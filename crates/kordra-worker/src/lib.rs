//! First real task executor: yt-dlp as a managed child process (no shell).
//! Full job cancellation, process-tree supervision and media selection follow later.
use anyhow::{bail, Context, Result};
use kordra_core::{Database, Download};
use kordra_engine_manager::YtDlpManager;
use kordra_torrent::TorrentEngine;
use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::{io::{AsyncBufReadExt, BufReader}, process::Command, sync::Semaphore};

#[derive(Clone)]
pub struct Worker {
    db: Database,
    manager: YtDlpManager,
    torrents: TorrentEngine,
    downloads: PathBuf,
    limit: Arc<Semaphore>,
}
impl Worker {
    pub fn new(db: Database, manager: YtDlpManager, downloads: PathBuf, concurrency: usize) -> Self {
        Self { db, manager, torrents: TorrentEngine::new(downloads.clone()), downloads, limit: Arc::new(Semaphore::new(concurrency.max(1))) }
    }
    pub fn enqueue(&self, item: Download) {
        let this = self.clone();
        tokio::spawn(async move {
            let id = item.id.clone();
            if let Err(error) = this.run(item).await {
                tracing::error!(task_id=%id, "download failed: {error:#}");
                if let Err(db_error) = this.db.finish(&id, false, Some(&error.to_string())).await {
                    tracing::error!(task_id=%id, "could not persist failure: {db_error:#}");
                }
            }
        });
    }
    pub async fn torrent_details(&self, id: &str) -> Result<serde_json::Value> {
        self.torrents.details(id).await.context("torrent is not active in this process")
    }
    /// Pause queued tasks, or active librqbit torrents. yt-dlp process suspension is planned.
    pub async fn pause(&self, id: &str) -> Result<Download> {
        let item = self.db.get(id).await?.context("task not found")?;
        match item.status.as_str() {
            "queued" => {
                if !self.db.transition(id, "queued", "paused").await? {
                    bail!("task changed state before it could be paused");
                }
            }
            "running" if item.engine == "librqbit" => {
                self.torrents.pause_task(id).await?;
                if !self.db.transition(id, "running", "paused").await? {
                    let _ = self.torrents.resume_task(id).await;
                    bail!("task changed state while pausing");
                }
            }
            _ => bail!("pause is not supported for this task state or engine"),
        }
        self.db.get(id).await?.context("task not found")
    }

    /// Resumes a paused torrent within the live session, or requeues a paused task.
    pub async fn resume(&self, id: &str) -> Result<Download> {
        let item = self.db.get(id).await?.context("task not found")?;
        if item.status != "paused" { bail!("task is not paused"); }
        if item.engine == "librqbit" && self.torrents.has_task(id).await {
            self.torrents.resume_task(id).await?;
            if !self.db.transition(id, "paused", "running").await? {
                let _ = self.torrents.pause_task(id).await;
                bail!("task changed state during resume");
            }
        } else {
            if !self.db.transition(id, "paused", "queued").await? { bail!("task changed state"); }
            self.enqueue(item);
        }
        self.db.get(id).await?.context("task not found")
    }

    pub async fn enqueue_pending(&self) -> Result<()> {
        for task in self.db.list().await? {
            if task.status == "queued" { self.enqueue(task); }
        }
        Ok(())
    }
    async fn run(&self, task: Download) -> Result<()> {
        let _permit = self.limit.acquire().await?;
        if !self.db.set_running(&task.id).await? { return Ok(()); }
        match task.engine.as_str() {
            "yt-dlp" => self.run_ytdlp(task).await,
            "librqbit" => self.run_torrent(task).await,
            other => anyhow::bail!("unsupported engine: {other}"),
        }
    }
    async fn run_torrent(&self, task: Download) -> Result<()> {
        let handle = self.torrents.add(&task.id, &task.source).await?;
        let mut ticks = tokio::time::interval(Duration::from_secs(2));
        let completed = handle.wait_until_completed();
        tokio::pin!(completed);
        loop {
            tokio::select! {
                result = &mut completed => {
                    result?;
                    let stats = handle.stats();
                    self.db.observe_transfer(&task.id, stats.progress_bytes, stats.uploaded_bytes, stats.total_bytes).await?;
                    self.db.finish(&task.id, true, None).await?;
                    break;
                }
                _ = ticks.tick() => {
                    let stats = handle.stats();
                    self.db.observe_transfer(&task.id, stats.progress_bytes, stats.uploaded_bytes, stats.total_bytes).await?;
                }
            }
        }
        // The rqbit session continues seeding until Kordra exits. Upload counters remain visible.
        let db = self.db.clone();
        let id = task.id.clone();
        let seeding_handle = handle.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                let stats = seeding_handle.stats();
                if let Err(error) = db.observe_transfer(&id, stats.progress_bytes, stats.uploaded_bytes, stats.total_bytes).await {
                    tracing::warn!(task_id=%id, "seeding stats unavailable: {error:#}");
                    break;
                }
            }
        });
        Ok(())
    }
    async fn run_ytdlp(&self, task: Download) -> Result<()> {
        let binary = self.manager.ensure_active().await?;
        tokio::fs::create_dir_all(&self.downloads).await?;
        let mut child = Command::new(binary)
            .arg("--ignore-config")
            .arg("--no-playlist")
            .arg("--newline")
            .arg("--progress-template")
            .arg("download:%(progress.downloaded_bytes)s")
            .arg("--paths")
            .arg(&self.downloads)
            .arg("--")
            .arg(&task.source)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("launching managed yt-dlp")?;
        let stderr = child.stderr.take().context("missing yt-dlp stderr")?;
        let stdout = child.stdout.take().context("missing yt-dlp stdout")?;
        let db_err = self.db.clone();
        let id_err = task.id.clone();
        let err_reader = tokio::spawn(async move { scan_progress(stderr, &db_err, &id_err).await; });
        let db_out = self.db.clone();
        let id_out = task.id.clone();
        let out_reader = tokio::spawn(async move { scan_progress(stdout, &db_out, &id_out).await; });
        let status = child.wait().await?;
        let _ = tokio::join!(err_reader, out_reader);
        if status.success() {
            self.db.finish(&task.id, true, None).await?;
        } else {
            anyhow::bail!("yt-dlp exited with {status}");
        }
        Ok(())
    }
}
fn parse_progress(line: &str) -> Option<u64> {
    let value = line.split("download:").nth(1)?.trim();
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() { None } else { digits.parse().ok() }
}
async fn scan_progress<R: tokio::io::AsyncRead + Unpin>(reader: R, db: &Database, task_id: &str) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(bytes) = parse_progress(&line) {
            if let Err(e) = db.observe_received(task_id, bytes).await { tracing::warn!("progress write failed: {e:#}"); }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_progress() {
        assert_eq!(parse_progress("[download] download:12345"), Some(12345));
        assert_eq!(parse_progress("download:NA"), None);
    }
}
