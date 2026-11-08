//! First real task executor: yt-dlp as a managed child process (no shell).
//! Full job cancellation, process-tree supervision and media selection follow later.
mod lifecycle;
pub use lifecycle::{ColdRetention, EngineLifecyclePolicy, PolicyStore, StartupMode};
use anyhow::{bail, Context, Result};
use kordra_core::{Database, Download};
use kordra_engine_manager::{DenoManager, YtDlpManager};
use kordra_torrent::TorrentEngine;
use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::{io::{AsyncBufReadExt, BufReader}, process::Command, sync::Semaphore};

#[derive(Clone)]
pub struct Worker {
    db: Database,
    manager: YtDlpManager,
    deno: DenoManager,
    torrents: TorrentEngine,
    downloads: PathBuf,
    lifecycle: PolicyStore,
    limit: Arc<Semaphore>,
}
impl Worker {
    pub fn new(db: Database, manager: YtDlpManager, deno: DenoManager, downloads: PathBuf, lifecycle: PolicyStore, concurrency: usize) -> Self {
        Self { db, manager, deno, torrents: TorrentEngine::new(downloads.clone()), downloads,
            lifecycle, limit: Arc::new(Semaphore::new(concurrency.max(1))) }
    }
    pub async fn lifecycle_policy(&self) -> EngineLifecyclePolicy { self.lifecycle.get().await }
    pub async fn lifecycle_status(&self) -> Result<serde_json::Value> {
        let torrent_ready = self.torrents.is_ready().await;
        let ytdlp = self.manager.state().await?;
        let deno = self.deno.state().await?;
        Ok(serde_json::json!({
            "policy": self.lifecycle.get().await,
            "engines": [
                {"id":"librqbit","kind":"embedded_service","ready":torrent_ready,
                    "active_tasks":self.torrents.active_count().await},
                {"id":"yt-dlp","kind":"short_lived_cli","ready":ytdlp.active.is_some(),
                    "active_version":ytdlp.active,"persistent_process_supported":false},
                {"id":"deno","kind":"runtime_dependency","ready":deno.active.is_some(),
                    "active_version":deno.active,"persistent_process_supported":false}
            ]
        }))
    }
    pub async fn apply_startup_policy(&self) -> Result<()> {
        if self.lifecycle.get().await.startup == StartupMode::Hot {
            self.torrents.warm().await?;
            // A hot start only warms already installed tools. It never downloads
            // missing executables or leaves invalid idle yt-dlp subprocesses.
            let _ = self.manager.active_path().await?;
            let _ = self.deno.active_path().await?;
        }
        Ok(())
    }
    pub async fn set_lifecycle_policy(&self, policy: EngineLifecyclePolicy) -> Result<EngineLifecyclePolicy> {
        self.lifecycle.update(policy).await?;
        if policy.startup == StartupMode::Hot {
            self.apply_startup_policy().await?;
        } else if !policy.retain_after_use() {
            self.torrents.cool_if_idle().await;
        }
        Ok(policy)
    }
    pub async fn prepare_engine(&self, name: &str) -> Result<serde_json::Value> {
        match name {
            "librqbit" => self.torrents.warm().await?,
            "yt-dlp" => { let _ = self.manager.ensure_active().await?; },
            "deno" => { let _ = self.deno.ensure_active().await?; },
            _ => bail!("unsupported engine or runtime: {name}"),
        }
        self.lifecycle_status().await
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
        if !self.lifecycle.get().await.retain_after_use() {
            // Temporary cold mode stops seeding and releases the session once the
            // last active torrent finishes. Persistent/hot mode continues seeding.
            self.torrents.finish_temporary(&task.id).await?;
            return Ok(());
        }
        // Persistent and hot modes allow completed torrents to seed until exit.
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
        let mut command = Command::new(binary);
        if youtube_url(&task.source) {
            let deno = self.deno.ensure_active().await.context("installing Deno required for YouTube")?;
            command.arg("--js-runtimes").arg(format!("deno:{}", deno.display()));
        }
        let mut child = command
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
fn youtube_url(source: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(source) else { return false; };
    url.host_str().is_some_and(|host| {
        host == "youtu.be" || host == "youtube.com" || host.ends_with(".youtube.com")
            || host == "youtube-nocookie.com" || host.ends_with(".youtube-nocookie.com")
    })
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
        assert!(youtube_url("https://www.youtube.com/watch?v=123"));
        assert!(youtube_url("https://youtu.be/test"));
        assert!(!youtube_url("https://youtube.com.evil.test/watch"));
        assert!(!youtube_url("magnet:?xt=urn:btih:foo"));
    }
}
