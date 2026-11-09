//! Durable task model and monotonically updated transfer counters.
use anyhow::{bail, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions}, FromRow, SqlitePool};
use std::{path::Path, time::Duration};
use uuid::Uuid;

#[derive(Clone)]
pub struct Database { pool: SqlitePool }

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Download {
    pub id: String,
    pub source: String,
    pub engine: String,
    pub status: String,
    pub downloaded_bytes: i64,
    pub uploaded_bytes: i64,
    pub total_bytes: i64,
    pub attempts: i64,
    pub priority: i64,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct NewDownload { pub source: String, pub engine: String }

#[derive(Debug, Serialize, FromRow)]
pub struct Statistics {
    pub total_downloaded_bytes: i64,
    pub total_uploaded_bytes: i64,
    pub completed_tasks: i64,
    pub failed_tasks: i64,
    pub total_tasks: i64,
    pub active_tasks: i64,
}

#[derive(Debug, Serialize, FromRow)]
pub struct DailyTransfer {
    pub day: String,
    pub downloaded_bytes: i64,
    pub uploaded_bytes: i64,
}
#[derive(Debug, Serialize, FromRow)]
pub struct EngineTransfer {
    pub engine: String,
    pub total_tasks: i64,
    pub downloaded_bytes: i64,
    pub uploaded_bytes: i64,
    pub completed_tasks: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentPreferences {
    /// On restart, seed completed torrents in hot or cold/persistent mode.
    pub seed_enabled: bool,
    /// None means download all files.
    pub only_files: Option<Vec<usize>>,
}
impl Default for TorrentPreferences {
    fn default() -> Self { Self { seed_enabled: true, only_files: None } }
}

impl Database {
    pub async fn open(data_dir: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(data_dir).await?;
        let options = SqliteConnectOptions::new().filename(data_dir.join("kordra.db"))
            .create_if_missing(true).journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true).busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new().max_connections(5).connect_with(options).await?;
        sqlx::query(r#"CREATE TABLE IF NOT EXISTS downloads (
            id TEXT PRIMARY KEY, source TEXT NOT NULL, engine TEXT NOT NULL,
            status TEXT NOT NULL, downloaded_bytes INTEGER NOT NULL DEFAULT 0,
            uploaded_bytes INTEGER NOT NULL DEFAULT 0, error TEXT,
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL
        )"#).execute(&pool).await?;
        if sqlx::query("SELECT name FROM pragma_table_info('downloads') WHERE name = 'total_bytes'")
            .fetch_optional(&pool).await?.is_none() {
            sqlx::query("ALTER TABLE downloads ADD COLUMN total_bytes INTEGER NOT NULL DEFAULT 0")
                .execute(&pool).await?;
        }
        if sqlx::query("SELECT name FROM pragma_table_info('downloads') WHERE name = 'attempts'")
            .fetch_optional(&pool).await?.is_none() {
            sqlx::query("ALTER TABLE downloads ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0")
                .execute(&pool).await?;
        }
        if sqlx::query("SELECT name FROM pragma_table_info('downloads') WHERE name = 'priority'")
            .fetch_optional(&pool).await?.is_none() {
            sqlx::query("ALTER TABLE downloads ADD COLUMN priority INTEGER NOT NULL DEFAULT 0")
                .execute(&pool).await?;
        }
        sqlx::query(r#"CREATE INDEX IF NOT EXISTS idx_download_queue
            ON downloads(status, priority DESC, created_at, id)"#).execute(&pool).await?;
        sqlx::query(r#"CREATE TABLE IF NOT EXISTS transfer_totals (
            id INTEGER PRIMARY KEY CHECK(id = 1),
            total_downloaded_bytes INTEGER NOT NULL DEFAULT 0,
            total_uploaded_bytes INTEGER NOT NULL DEFAULT 0,
            completed_tasks INTEGER NOT NULL DEFAULT 0,
            failed_tasks INTEGER NOT NULL DEFAULT 0
        )"#).execute(&pool).await?;
        sqlx::query("INSERT OR IGNORE INTO transfer_totals(id) VALUES (1)").execute(&pool).await?;
        sqlx::query(r#"CREATE TABLE IF NOT EXISTS daily_transfers (
            day TEXT PRIMARY KEY,
            downloaded_bytes INTEGER NOT NULL DEFAULT 0,
            uploaded_bytes INTEGER NOT NULL DEFAULT 0
        )"#).execute(&pool).await?;
        if sqlx::query("SELECT name FROM pragma_table_info('daily_transfers') WHERE name = 'uploaded_bytes'")
            .fetch_optional(&pool).await?.is_none() {
            sqlx::query("ALTER TABLE daily_transfers ADD COLUMN uploaded_bytes INTEGER NOT NULL DEFAULT 0")
                .execute(&pool).await?;
        }
        sqlx::query(r#"CREATE TABLE IF NOT EXISTS torrent_preferences (
            task_id TEXT PRIMARY KEY REFERENCES downloads(id) ON DELETE CASCADE,
            seed_enabled INTEGER NOT NULL DEFAULT 1 CHECK(seed_enabled IN (0, 1)),
            only_files TEXT
        )"#).execute(&pool).await?;
        // Neither a running download nor a pending cancellation can remain active
        // inside this *new* process. Both need explicit recovery after a crash.
        sqlx::query("UPDATE downloads SET status = 'interrupted', updated_at = ? WHERE status IN ('running', 'cancelling')")
            .bind(Utc::now().to_rfc3339()).execute(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn list(&self) -> Result<Vec<Download>> {
        Ok(sqlx::query_as::<_, Download>("SELECT * FROM downloads ORDER BY created_at DESC LIMIT 500")
            .fetch_all(&self.pool).await?)
    }
    /// Retrieve the entire startup backlog in FIFO order, not only the last 500 UI rows.
    pub async fn completed_torrents_to_seed(&self) -> Result<Vec<Download>> {
        Ok(sqlx::query_as::<_, Download>(r#"
            SELECT d.* FROM downloads AS d
            LEFT JOIN torrent_preferences AS p ON p.task_id = d.id
            WHERE d.engine = 'librqbit' AND d.status = 'completed'
              AND COALESCE(p.seed_enabled, 1) = 1
            ORDER BY d.created_at ASC, d.id ASC
        "#).fetch_all(&self.pool).await?)
    }
    /// Returns None for an unknown or non-torrent task.
    pub async fn torrent_preferences(&self, id: &str) -> Result<Option<TorrentPreferences>> {
        let row = sqlx::query(r#"
            SELECT p.seed_enabled, p.only_files FROM downloads AS d
            LEFT JOIN torrent_preferences AS p ON p.task_id = d.id
            WHERE d.id = ? AND d.engine = 'librqbit'
        "#).bind(id).fetch_optional(&self.pool).await?;
        use sqlx::Row;
        row.map(|r| {
            let seed_enabled: Option<i64> = r.try_get(0)?;
            let selection: Option<String> = r.try_get(1)?;
            Ok(TorrentPreferences {
                seed_enabled: seed_enabled.unwrap_or(1) != 0,
                only_files: selection.map(|raw| serde_json::from_str::<Vec<usize>>(&raw))
                    .transpose()?,
            })
        }).transpose()
    }
    /// Store an active torrent's file selection; never accepts an empty selection.
    pub async fn save_torrent_selection(&self, id: &str, only_files: &[usize]) -> Result<()> {
        if only_files.is_empty() || only_files.len() > 10_000 {
            bail!("invalid torrent file selection");
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO torrent_preferences(task_id) SELECT id FROM downloads WHERE id = ? AND engine = 'librqbit'")
            .bind(id).execute(&mut *tx).await?;
        let updated = sqlx::query("UPDATE torrent_preferences SET only_files = ? WHERE task_id = ?")
            .bind(serde_json::to_string(only_files)?).bind(id)
            .execute(&mut *tx).await?;
        if updated.rows_affected() != 1 { bail!("torrent not found"); }
        tx.commit().await?;
        Ok(())
    }
    /// Persistently disable automatic seeding of a completed torrent.
    pub async fn disable_torrent_seeding(&self, id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO torrent_preferences(task_id) SELECT id FROM downloads WHERE id = ? AND engine = 'librqbit' AND status = 'completed'")
            .bind(id).execute(&mut *tx).await?;
        let updated = sqlx::query("UPDATE torrent_preferences SET seed_enabled = 0 WHERE task_id = ? AND seed_enabled = 1")
            .bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(updated.rows_affected() == 1)
    }
    pub async fn list_queued(&self) -> Result<Vec<Download>> {
        Ok(sqlx::query_as::<_, Download>(
            "SELECT * FROM downloads WHERE status = 'queued' ORDER BY created_at ASC, id ASC"
        ).fetch_all(&self.pool).await?)
    }
    pub async fn get(&self, id: &str) -> Result<Option<Download>> {
        Ok(sqlx::query_as::<_, Download>("SELECT * FROM downloads WHERE id = ?")
            .bind(id).fetch_optional(&self.pool).await?)
    }
    pub async fn create(&self, req: NewDownload) -> Result<Download> {
        if req.source.trim().is_empty() || req.source.len() > 4096 { bail!("invalid source"); }
        if req.engine != "yt-dlp" && req.engine != "librqbit" { bail!("engine is not implemented yet"); }
        let http = req.source.starts_with("https://") || req.source.starts_with("http://");
        if (req.engine == "yt-dlp" && !http)
            || (req.engine == "librqbit" && !http && !req.source.starts_with("magnet:?")) {
            bail!("unsupported source for requested engine");
        }
        let now = Utc::now().to_rfc3339();
        let task = Download { id: Uuid::new_v4().to_string(), source: req.source,
            engine: req.engine, status: "queued".into(), downloaded_bytes: 0,
            uploaded_bytes: 0, total_bytes: 0, attempts: 0, priority: 0, error: None, created_at: now.clone(), updated_at: now };
        sqlx::query("INSERT INTO downloads(id, source, engine, status, downloaded_bytes, uploaded_bytes, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&task.id).bind(&task.source).bind(&task.engine).bind(&task.status)
            .bind(task.downloaded_bytes).bind(task.uploaded_bytes).bind(&task.created_at)
            .bind(&task.updated_at).execute(&self.pool).await?;
        Ok(task)
    }

    pub async fn transition(&self, id: &str, from: &str, to: &str) -> Result<bool> {
        // Internal state machine operation. Callers must validate the allowed transition.
        let result = sqlx::query("UPDATE downloads SET status = ?, updated_at = ? WHERE id = ? AND status = ?")
            .bind(to).bind(Utc::now().to_rfc3339()).bind(id).bind(from)
            .execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }
    /// Atomically requeue an interrupted or failed task. Duplicate API retries cannot schedule
    /// the same task twice because only one caller wins the state transition.
    pub async fn requeue_failed(&self, id: &str) -> Result<Option<Download>> {
        let updated = sqlx::query("UPDATE downloads SET status = 'queued', error = NULL, updated_at = ? WHERE id = ? AND status IN ('failed', 'interrupted')")
            .bind(Utc::now().to_rfc3339()).bind(id).execute(&self.pool).await?;
        if updated.rows_affected() == 0 { return Ok(None); }
        self.get(id).await
    }
    /// Complete an acknowledged cancellation without recording a task failure.
    pub async fn finish_cancelled(&self, id: &str) -> Result<bool> {
        self.transition(id, "cancelling", "cancelled").await
    }
    /// Single atomic FIFO claim, so concurrent workers never execute the same job.
    /// SQLite RETURNING returns the *updated* attempt number and persisted
    /// progress, preventing stale snapshots from resetting retry counters.
    /// Set a queued or paused task's priority. A running job is never
    /// interrupted or moved when its priority changes.
    pub async fn set_priority(&self, id: &str, value: i64) -> Result<Option<Download>> {
        if !(-2..=2).contains(&value) {
            bail!("priority must be between -2 and 2");
        }
        Ok(sqlx::query_as::<_, Download>(r#"
            UPDATE downloads SET priority = ?, updated_at = ?
            WHERE id = ? AND status IN ('queued', 'paused')
            RETURNING *
        "#)
            .bind(value).bind(Utc::now().to_rfc3339()).bind(id)
            .fetch_optional(&self.pool).await?)
    }
    pub async fn claim_next(&self) -> Result<Option<Download>> {
        Ok(sqlx::query_as::<_, Download>(r#"
            UPDATE downloads
            SET status = 'running', attempts = attempts + 1, error = NULL, updated_at = ?
            WHERE id = (
                SELECT id FROM downloads WHERE status = 'queued'
                ORDER BY priority DESC, created_at ASC, id ASC LIMIT 1
            ) AND status = 'queued'
            RETURNING *
        "#)
            .bind(Utc::now().to_rfc3339())
            .fetch_optional(&self.pool).await?)
    }
    pub async fn queue_counts(&self) -> Result<(i64, i64, i64)> {
        use sqlx::Row;
        let row = sqlx::query(r#"
            SELECT
                COUNT(*) FILTER (WHERE status = 'queued'),
                COUNT(*) FILTER (WHERE status = 'running'),
                COUNT(*) FILTER (WHERE status = 'paused')
            FROM downloads
        "#).fetch_one(&self.pool).await?;
        Ok((row.try_get(0)?, row.try_get(1)?, row.try_get(2)?))
    }
    pub async fn set_running(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("UPDATE downloads SET status = 'running', attempts = attempts + 1, error = NULL, updated_at = ? WHERE id = ? AND status = 'queued'")
            .bind(Utc::now().to_rfc3339()).bind(id).execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }
    pub async fn observe_received(&self, id: &str, observed: u64) -> Result<()> {
        self.observe_transfer(id, observed, 0, 0).await
    }
    pub async fn observe_transfer(&self, id: &str, downloaded: u64, uploaded: u64, total: u64) -> Result<()> {
        let downloaded = i64::try_from(downloaded)?;
        let uploaded = i64::try_from(uploaded)?;
        let total = i64::try_from(total)?;
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query("SELECT downloaded_bytes, uploaded_bytes FROM downloads WHERE id = ? AND status IN ('running', 'paused', 'completed')")
            .bind(id).fetch_optional(&mut *tx).await?;
        let Some(row) = result else { bail!("running task not found"); };
        use sqlx::Row;
        let previous: i64 = row.try_get(0)?;
        let previous_upload: i64 = row.try_get(1)?;
        let delta_download = downloaded.saturating_sub(previous);
        let delta_upload = uploaded.saturating_sub(previous_upload);
        sqlx::query("UPDATE downloads SET downloaded_bytes = MAX(downloaded_bytes, ?), uploaded_bytes = MAX(uploaded_bytes, ?), total_bytes = MAX(total_bytes, ?), updated_at = ? WHERE id = ?")
            .bind(downloaded).bind(uploaded).bind(total).bind(Utc::now().to_rfc3339()).bind(id)
            .execute(&mut *tx).await?;
        if delta_download > 0 || delta_upload > 0 {
            sqlx::query("UPDATE transfer_totals SET total_downloaded_bytes = total_downloaded_bytes + ?, total_uploaded_bytes = total_uploaded_bytes + ? WHERE id = 1")
                .bind(delta_download).bind(delta_upload).execute(&mut *tx).await?;
            sqlx::query(r#"INSERT INTO daily_transfers(day, downloaded_bytes, uploaded_bytes) VALUES(?, ?, ?)
                ON CONFLICT(day) DO UPDATE SET
                downloaded_bytes = downloaded_bytes + excluded.downloaded_bytes,
                uploaded_bytes = uploaded_bytes + excluded.uploaded_bytes"#)
                .bind(Utc::now().format("%Y-%m-%d").to_string()).bind(delta_download).bind(delta_upload)
                .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn finish(&self, id: &str, success: bool, error: Option<&str>) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query("UPDATE downloads SET status = ?, error = ?, updated_at = ? WHERE id = ? AND status IN ('running', 'paused')")
            .bind(if success { "completed" } else { "failed" })
            .bind(error).bind(Utc::now().to_rfc3339()).bind(id).execute(&mut *tx).await?;
        if result.rows_affected() == 1 {
            let field = if success { "completed_tasks" } else { "failed_tasks" };
            let sql = format!("UPDATE transfer_totals SET {field} = {field} + 1 WHERE id = 1");
            sqlx::query(&sql).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn daily_stats(&self) -> Result<Vec<DailyTransfer>> {
        Ok(sqlx::query_as::<_, DailyTransfer>(
            "SELECT day, downloaded_bytes, uploaded_bytes FROM daily_transfers ORDER BY day DESC LIMIT 365"
        ).fetch_all(&self.pool).await?)
    }
    pub async fn engine_stats(&self) -> Result<Vec<EngineTransfer>> {
        Ok(sqlx::query_as::<_, EngineTransfer>(r#"SELECT engine,
            COUNT(*) AS total_tasks,
            COALESCE(SUM(downloaded_bytes), 0) AS downloaded_bytes,
            COALESCE(SUM(uploaded_bytes), 0) AS uploaded_bytes,
            SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) AS completed_tasks
            FROM downloads GROUP BY engine ORDER BY downloaded_bytes DESC"#)
            .fetch_all(&self.pool).await?)
    }
    pub async fn stats(&self) -> Result<Statistics> {
        Ok(sqlx::query_as::<_, Statistics>(r#"SELECT t.total_downloaded_bytes,
            t.total_uploaded_bytes, t.completed_tasks, t.failed_tasks,
            (SELECT COUNT(*) FROM downloads) AS total_tasks,
            (SELECT COUNT(*) FROM downloads WHERE status = 'running') AS active_tasks
            FROM transfer_totals t WHERE t.id = 1"#).fetch_one(&self.pool).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn persists_and_deduplicates_progress() {
        let dir = std::env::temp_dir().join(format!("kordra-test-{}", Uuid::new_v4()));
        let db = Database::open(&dir).await.unwrap();
        let task = db.create(NewDownload { source: "https://example.org/video".into(), engine: "yt-dlp".into() }).await.unwrap();
        assert!(db.set_running(&task.id).await.unwrap());
        assert!(!db.set_running(&task.id).await.unwrap());
        db.observe_received(&task.id, 100).await.unwrap();
        db.observe_received(&task.id, 100).await.unwrap();
        db.observe_received(&task.id, 80).await.unwrap();
        db.finish(&task.id, true, None).await.unwrap();
        db.finish(&task.id, true, None).await.unwrap();
        db.observe_transfer(&task.id, 100, 250, 1000).await.unwrap();
        db.observe_transfer(&task.id, 100, 250, 1000).await.unwrap();
        let s = db.stats().await.unwrap();
        assert_eq!(s.total_downloaded_bytes, 100);
        assert_eq!(s.total_uploaded_bytes, 250);
        assert_eq!(s.completed_tasks, 1);
        assert_eq!(db.get(&task.id).await.unwrap().unwrap().attempts, 1);
        assert_eq!(s.total_tasks, 1);
        assert_eq!(db.daily_stats().await.unwrap()[0].downloaded_bytes, 100);
        assert_eq!(db.daily_stats().await.unwrap()[0].uploaded_bytes, 250);
        assert_eq!(db.engine_stats().await.unwrap()[0].completed_tasks, 1);
        drop(db);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn torrent_preferences_survive_restarts_and_filter_seed_candidates() {
        let dir = std::env::temp_dir().join(format!("kordra-torrent-prefs-{}", Uuid::new_v4()));
        let db = Database::open(&dir).await.unwrap();
        let a = db.create(NewDownload { source: "magnet:?xt=urn:btih:aaa".into(), engine: "librqbit".into() }).await.unwrap();
        let b = db.create(NewDownload { source: "magnet:?xt=urn:btih:bbb".into(), engine: "librqbit".into() }).await.unwrap();
        assert_eq!(db.torrent_preferences(&a.id).await.unwrap().unwrap(), TorrentPreferences::default());
        assert!(db.set_running(&a.id).await.unwrap());
        assert!(db.set_running(&b.id).await.unwrap());
        db.finish(&a.id, true, None).await.unwrap();
        db.finish(&b.id, true, None).await.unwrap();
        db.save_torrent_selection(&a.id, &[0, 2, 4]).await.unwrap();
        assert!(db.disable_torrent_seeding(&b.id).await.unwrap());
        assert!(!db.disable_torrent_seeding(&b.id).await.unwrap());
        assert_eq!(db.completed_torrents_to_seed().await.unwrap().len(), 1);
        drop(db);
        let reopened = Database::open(&dir).await.unwrap();
        assert_eq!(reopened.completed_torrents_to_seed().await.unwrap()[0].id, a.id);
        assert_eq!(reopened.torrent_preferences(&a.id).await.unwrap().unwrap().only_files, Some(vec![0, 2, 4]));
        assert!(!reopened.torrent_preferences(&b.id).await.unwrap().unwrap().seed_enabled);
        assert!(reopened.save_torrent_selection("unknown", &[0]).await.is_err());
        drop(reopened);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn pending_cancellation_becomes_interrupted_on_restart() {
        let dir = std::env::temp_dir().join(format!("kordra-cancelling-{}", Uuid::new_v4()));
        let db = Database::open(&dir).await.unwrap();
        let item = db.create(NewDownload { source: "https://example.org/test".into(), engine: "yt-dlp".into() }).await.unwrap();
        assert!(db.set_running(&item.id).await.unwrap());
        assert!(db.transition(&item.id, "running", "cancelling").await.unwrap());
        drop(db);
        let reopened = Database::open(&dir).await.unwrap();
        assert_eq!(reopened.get(&item.id).await.unwrap().unwrap().status, "interrupted");
        drop(reopened);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn cancellation_transitions_preserve_correct_counts() {
        let dir = std::env::temp_dir().join(format!("kordra-cancel-{}", Uuid::new_v4()));
        let db = Database::open(&dir).await.unwrap();
        let queued = db.create(NewDownload { source: "https://example.org/q".into(), engine: "yt-dlp".into() }).await.unwrap();
        assert!(db.transition(&queued.id, "queued", "cancelled").await.unwrap());
        assert!(!db.set_running(&queued.id).await.unwrap());
        let running = db.create(NewDownload { source: "https://example.org/r".into(), engine: "yt-dlp".into() }).await.unwrap();
        assert!(db.set_running(&running.id).await.unwrap());
        assert!(db.transition(&running.id, "running", "cancelling").await.unwrap());
        assert!(db.finish_cancelled(&running.id).await.unwrap());
        assert!(!db.finish_cancelled(&running.id).await.unwrap());
        let totals = db.stats().await.unwrap();
        assert_eq!(totals.completed_tasks, 0);
        assert_eq!(totals.failed_tasks, 0);
        assert_eq!(db.get(&running.id).await.unwrap().unwrap().status, "cancelled");
        drop(db);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn atomic_fifo_claim_never_duplicates_under_concurrency() {
        use std::collections::HashSet;
        let dir = std::env::temp_dir().join(format!("kordra-claim-{}", Uuid::new_v4()));
        let db = Database::open(&dir).await.unwrap();
        let mut original = Vec::new();
        for index in 0..36 {
            let item = db.create(NewDownload {
                source: format!("https://example.org/claim/{index}"),
                engine: "yt-dlp".into()
            }).await.unwrap();
            original.push(item.id);
        }
        let work = (0..12).map(|_| {
            let db = db.clone();
            tokio::spawn(async move {
                let mut claimed = Vec::new();
                while let Some(item) = db.claim_next().await.unwrap() {
                    assert_eq!(item.status, "running");
                    assert_eq!(item.attempts, 1);
                    claimed.push(item.id);
                }
                claimed
            })
        });
        let mut all = Vec::new();
        for job in work { all.extend(job.await.unwrap()); }
        let unique: HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), original.len());
        assert_eq!(all.len(), original.len());
        let (queued, active, paused) = db.queue_counts().await.unwrap();
        assert_eq!((queued, active, paused), (0, 36, 0));
        assert!(db.claim_next().await.unwrap().is_none());
        drop(db);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn queue_priorities_are_persistent_and_cannot_preempt_running_tasks() {
        let dir = std::env::temp_dir().join(format!("kordra-priority-{}", Uuid::new_v4()));
        let db = Database::open(&dir).await.unwrap();
        let low = db.create(NewDownload { source: "https://example.org/low".into(), engine: "yt-dlp".into() }).await.unwrap();
        let normal = db.create(NewDownload { source: "https://example.org/normal".into(), engine: "yt-dlp".into() }).await.unwrap();
        let high = db.create(NewDownload { source: "https://example.org/high".into(), engine: "yt-dlp".into() }).await.unwrap();
        assert_eq!(db.set_priority(&low.id, -2).await.unwrap().unwrap().priority, -2);
        assert_eq!(db.set_priority(&high.id, 2).await.unwrap().unwrap().priority, 2);
        assert!(db.set_priority(&normal.id, 3).await.is_err());
        assert!(db.set_priority("missing", 1).await.unwrap().is_none());
        let first = db.claim_next().await.unwrap().unwrap();
        assert_eq!(first.id, high.id);
        assert!(db.set_priority(&high.id, -1).await.unwrap().is_none());
        assert_eq!(db.claim_next().await.unwrap().unwrap().id, normal.id);
        assert_eq!(db.claim_next().await.unwrap().unwrap().id, low.id);
        drop(db);
        let reopened = Database::open(&dir).await.unwrap();
        assert_eq!(reopened.get(&high.id).await.unwrap().unwrap().priority, 2);
        assert_eq!(reopened.get(&low.id).await.unwrap().unwrap().priority, -2);
        drop(reopened);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn queue_cancel_and_pause_prevent_future_claims() {
        let dir = std::env::temp_dir().join(format!("kordra-claim-cancel-{}", Uuid::new_v4()));
        let db = Database::open(&dir).await.unwrap();
        let cancelled = db.create(NewDownload { source: "https://example.org/a".into(), engine: "yt-dlp".into() }).await.unwrap();
        let paused = db.create(NewDownload { source: "https://example.org/b".into(), engine: "yt-dlp".into() }).await.unwrap();
        let running = db.create(NewDownload { source: "https://example.org/c".into(), engine: "yt-dlp".into() }).await.unwrap();
        assert!(db.transition(&cancelled.id, "queued", "cancelled").await.unwrap());
        assert!(db.transition(&paused.id, "queued", "paused").await.unwrap());
        let item = db.claim_next().await.unwrap().unwrap();
        assert_eq!(item.id, running.id);
        assert!(db.claim_next().await.unwrap().is_none());
        assert!(db.transition(&paused.id, "paused", "queued").await.unwrap());
        let item = db.claim_next().await.unwrap().unwrap();
        assert_eq!(item.id, paused.id);
        assert_eq!(item.attempts, 1);
        assert_eq!(db.get(&cancelled.id).await.unwrap().unwrap().attempts, 0);
        drop(db);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn pending_backlog_is_not_limited_by_ui_pagination() {
        let dir = std::env::temp_dir().join(format!("kordra-backlog-{}", Uuid::new_v4()));
        let db = Database::open(&dir).await.unwrap();
        for index in 0..503 {
            db.create(NewDownload { source: format!("https://example.org/{index}"), engine: "yt-dlp".into() }).await.unwrap();
        }
        assert_eq!(db.list().await.unwrap().len(), 500);
        assert_eq!(db.list_queued().await.unwrap().len(), 503);
        drop(db);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn retry_is_atomic_and_attempts_are_persisted() {
        let dir = std::env::temp_dir().join(format!("kordra-retry-{}", Uuid::new_v4()));
        let db = Database::open(&dir).await.unwrap();
        let task = db.create(NewDownload {
            source: "https://example.org/retry".into(), engine: "yt-dlp".into()
        }).await.unwrap();

        assert!(db.requeue_failed(&task.id).await.unwrap().is_none());
        assert!(db.set_running(&task.id).await.unwrap());
        db.finish(&task.id, false, Some("failed to connect")).await.unwrap();
        let queued = db.requeue_failed(&task.id).await.unwrap().unwrap();
        assert_eq!(queued.status, "queued");
        assert!(queued.error.is_none());
        assert!(db.requeue_failed(&task.id).await.unwrap().is_none());
        assert!(db.set_running(&task.id).await.unwrap());
        assert_eq!(db.get(&task.id).await.unwrap().unwrap().attempts, 2);
        let s = db.stats().await.unwrap();
        assert_eq!(s.failed_tasks, 1);
        drop(db);
        let reopened = Database::open(&dir).await.unwrap();
        let persisted = reopened.get(&task.id).await.unwrap().unwrap();
        assert_eq!(persisted.status, "interrupted");
        assert_eq!(persisted.attempts, 2);
        assert!(reopened.requeue_failed(&task.id).await.unwrap().is_some());
        drop(reopened);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}
