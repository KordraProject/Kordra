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
}
#[derive(Debug, Serialize, FromRow)]
pub struct EngineTransfer {
    pub engine: String,
    pub total_tasks: i64,
    pub downloaded_bytes: i64,
    pub completed_tasks: i64,
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
        sqlx::query(r#"CREATE TABLE IF NOT EXISTS transfer_totals (
            id INTEGER PRIMARY KEY CHECK(id = 1),
            total_downloaded_bytes INTEGER NOT NULL DEFAULT 0,
            total_uploaded_bytes INTEGER NOT NULL DEFAULT 0,
            completed_tasks INTEGER NOT NULL DEFAULT 0,
            failed_tasks INTEGER NOT NULL DEFAULT 0
        )"#).execute(&pool).await?;
        sqlx::query("INSERT OR IGNORE INTO transfer_totals(id) VALUES (1)").execute(&pool).await?;
        sqlx::query(r#"CREATE TABLE IF NOT EXISTS daily_transfers (
            day TEXT PRIMARY KEY, downloaded_bytes INTEGER NOT NULL DEFAULT 0
        )"#).execute(&pool).await?;
        // Tasks interrupted by previous process termination are not silently retried.
        sqlx::query("UPDATE downloads SET status = 'interrupted', updated_at = ? WHERE status = 'running'")
            .bind(Utc::now().to_rfc3339()).execute(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn list(&self) -> Result<Vec<Download>> {
        Ok(sqlx::query_as::<_, Download>("SELECT * FROM downloads ORDER BY created_at DESC LIMIT 500")
            .fetch_all(&self.pool).await?)
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
            uploaded_bytes: 0, total_bytes: 0, error: None, created_at: now.clone(), updated_at: now };
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
    pub async fn set_running(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("UPDATE downloads SET status = 'running', error = NULL, updated_at = ? WHERE id = ? AND status = 'queued'")
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
            if delta_download > 0 {
                sqlx::query(r#"INSERT INTO daily_transfers(day, downloaded_bytes) VALUES(?, ?)
                    ON CONFLICT(day) DO UPDATE SET downloaded_bytes = downloaded_bytes + excluded.downloaded_bytes"#)
                    .bind(Utc::now().format("%Y-%m-%d").to_string()).bind(delta_download)
                    .execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn finish(&self, id: &str, success: bool, error: Option<&str>) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query("UPDATE downloads SET status = ?, error = ?, updated_at = ? WHERE id = ? AND status = 'running'")
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
            "SELECT day, downloaded_bytes FROM daily_transfers ORDER BY day DESC LIMIT 365"
        ).fetch_all(&self.pool).await?)
    }
    pub async fn engine_stats(&self) -> Result<Vec<EngineTransfer>> {
        Ok(sqlx::query_as::<_, EngineTransfer>(r#"SELECT engine,
            COUNT(*) AS total_tasks,
            COALESCE(SUM(downloaded_bytes), 0) AS downloaded_bytes,
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
        let s = db.stats().await.unwrap();
        assert_eq!(s.total_downloaded_bytes, 100);
        assert_eq!(s.total_uploaded_bytes, 0);
        assert_eq!(s.completed_tasks, 1);
        assert_eq!(s.total_tasks, 1);
        assert_eq!(db.daily_stats().await.unwrap()[0].downloaded_bytes, 100);
        assert_eq!(db.engine_stats().await.unwrap()[0].completed_tasks, 1);
        drop(db);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}
