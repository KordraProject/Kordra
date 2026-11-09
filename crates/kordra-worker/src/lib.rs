//! First real task executor: yt-dlp as a managed child process (no shell).
//! Cancellation of CLI jobs is supported; full process-tree supervision remains planned.
mod lifecycle;
mod transfer_settings;
pub use lifecycle::{ColdRetention, EngineLifecyclePolicy, PolicyStore, StartupMode};
pub use transfer_settings::{TransferSettings, TransferStore};
use transfer_settings::LiveSlots;
use anyhow::{bail, Context, Result};
use kordra_core::{Database, Download};
use kordra_engine_manager::{DenoManager, YtDlpManager};
use kordra_torrent::TorrentEngine;
use std::{collections::{HashMap, HashSet}, path::PathBuf, process::Stdio, sync::{atomic::{AtomicBool, Ordering}, Arc}, time::Duration};
use tokio::{io::{AsyncBufReadExt, BufReader}, process::Command, sync::{Mutex, Notify}};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Worker {
    db: Database,
    manager: YtDlpManager,
    deno: DenoManager,
    torrents: TorrentEngine,
    downloads: PathBuf,
    lifecycle: PolicyStore,
    running_cli: Arc<Mutex<HashMap<String, CancellationToken>>>,
    restoring_seeds: Arc<Mutex<HashSet<String>>>,
    queue_notify: Arc<Notify>,
    queue_started: Arc<AtomicBool>,
    transfer: TransferStore,
    slots: LiveSlots,
}
impl Worker {
    pub async fn new(db: Database, manager: YtDlpManager, deno: DenoManager, downloads: PathBuf,
        lifecycle: PolicyStore, transfer: TransferStore) -> Self {
        let settings = transfer.get().await;
        Self { db, manager, deno, torrents: TorrentEngine::new(downloads.clone()), downloads,
            lifecycle, running_cli: Arc::new(Mutex::new(HashMap::new())),
            restoring_seeds: Arc::new(Mutex::new(HashSet::new())),
            queue_notify: Arc::new(Notify::new()), queue_started: Arc::new(AtomicBool::new(false)),
            transfer, slots: LiveSlots::new(settings.concurrency) }
    }
    pub async fn queue_status(&self) -> Result<serde_json::Value> {
        let (queued, running, paused) = self.db.queue_counts().await?;
        Ok(serde_json::json!({
            "queued":queued, "running":running, "paused":paused,
            "max_concurrent":self.slots.limit(),
            "available_slots":self.slots.available(),
            "occupied_slots":self.slots.active()
        }))
    }
    pub async fn transfer_settings(&self) -> TransferSettings { self.transfer.get().await }
    pub async fn apply_transfer_settings(&self) -> Result<()> {
        let settings = self.transfer.get().await;
        self.torrents.set_rate_limits(settings.torrent_download_bps,
            settings.torrent_upload_bps).await;
        self.slots.set_limit(settings.concurrency);
        Ok(())
    }
    pub async fn set_transfer_settings(&self, settings: TransferSettings) -> Result<TransferSettings> {
        self.transfer.save(settings).await?;
        self.apply_transfer_settings().await?;
        Ok(settings)
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
        let old = self.lifecycle.get().await;
        self.lifecycle.update(policy).await?;
        if policy.retain_after_use() && !old.retain_after_use() {
            self.restore_completed_torrents().await?;
        }
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
    /// SQLite is the queue of record: signal fixed workers, never spawn a
    /// sleeping Tokio task for every queued download.
    pub fn enqueue(&self, _item: Download) {
        self.queue_notify.notify_one();
    }

    async fn execute_claimed(&self, item: Download) -> Result<()> {
        match item.engine.as_str() {
            "yt-dlp" => {
                let token = CancellationToken::new();
                {
                    // The cancellation endpoint takes this same mutex while it
                    // updates SQLite, closing the claim/registration race.
                    let mut handles = self.running_cli.lock().await;
                    match self.db.get(&item.id).await?.map(|record| record.status) {
                        Some(status) if status == "running" => {
                            handles.insert(item.id.clone(), token.clone());
                        }
                        Some(status) if status == "cancelling" => {
                            self.db.finish_cancelled(&item.id).await?;
                            return Ok(());
                        }
                        _ => return Ok(()),
                    }
                }
                let outcome = self.run_ytdlp(item.clone(), token).await;
                self.running_cli.lock().await.remove(&item.id);
                outcome
            }
            "librqbit" => self.run_torrent(item).await,
            other => bail!("unsupported engine: {other}"),
        }
    }

    async fn handle_task_error(&self, id: &str, error: anyhow::Error) {
        tracing::error!(task_id=%id, "download failed: {error:#}");
        // If a cancellation beat an engine error, finish it as cancelled.
        if let Ok(Some(task)) = self.db.get(id).await {
            if task.status == "cancelling" {
                let _ = self.db.finish_cancelled(id).await;
                return;
            }
        }
        if let Err(db_error) = self.db.finish(id, false, Some(&error.to_string())).await {
            tracing::error!(task_id=%id, "could not persist failure: {db_error:#}");
        }
    }

    /// Long-lived, fixed-size dispatcher loop. A registered notification
    /// precedes the database check, so no enqueue wake-up can be lost.
    async fn queue_worker_loop(&self) {
        loop {
            let wake = self.queue_notify.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            let permit = self.slots.acquire().await;
            match self.db.claim_next().await {
                Ok(Some(task)) => {
                    let id = task.id.clone();
                    if let Err(error) = self.execute_claimed(task).await {
                        self.handle_task_error(&id, error).await;
                    }
                }
                Ok(None) => {
                    drop(permit);
                    wake.await;
                }
                Err(error) => {
                    tracing::error!("could not claim queued download: {error:#}");
                    drop(permit);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    /// Cancel queued/paused tasks immediately, or request termination of a live
    /// yt-dlp subprocess. Active torrents require a safe deletion implementation.
    pub async fn cancel(&self, id: &str) -> Result<Download> {
        let item = self.db.get(id).await?.context("task not found")?;
        match item.status.as_str() {
            "queued" => {
                if !self.db.transition(id, "queued", "cancelled").await? {
                    bail!("task state changed before cancellation");
                }
            }
            "paused" if !self.torrents.has_task(id).await => {
                if !self.db.transition(id, "paused", "cancelled").await? {
                    bail!("task state changed before cancellation");
                }
            }
            "running" if item.engine == "yt-dlp" => {
                // Hold the registration lock through the SQLite transition.
                // If the worker has not registered yet, it will notice the
                // cancelling state before launching any subprocess.
                let handles = self.running_cli.lock().await;
                if !self.db.transition(id, "running", "cancelling").await? {
                    bail!("task state changed before cancellation");
                }
                if let Some(token) = handles.get(id) { token.cancel(); }
            }
            "running" | "paused" if item.engine == "librqbit" => {
                if !self.torrents.has_task(id).await {
                    bail!("torrent is not yet active (magnet resolution may still be running)");
                }
                if let Some((down, up, size)) = self.torrents.transfer_snapshot(id).await {
                    self.db.observe_transfer(id, down, up, size).await?;
                }
                if !self.db.transition(id, &item.status, "cancelling").await? {
                    bail!("task state changed before cancellation");
                }
                if let Err(error) = self.torrents.forget_task(id).await {
                    let _ = self.db.transition(id, "cancelling", &item.status).await;
                    return Err(error);
                }
                self.db.finish_cancelled(id).await?;
                if !self.lifecycle.get().await.retain_after_use() {
                    self.torrents.cool_if_idle().await;
                }
            }
            _ => bail!("cancellation is unsupported for this engine or task state"),
        }
        self.db.get(id).await?.context("task not found")
    }

    /// Stop seeding without changing successful task history or deleting files.
    pub async fn stop_seeding(&self, id: &str) -> Result<Download> {
        let task = self.db.get(id).await?.context("task not found")?;
        if task.engine != "librqbit" || task.status != "completed" {
            bail!("only completed and actively seeding torrents can stop seeding");
        }
        // Persist the user's decision first so a crash or in-flight restore
        // cannot make the completed torrent restart uploading on next launch.
        self.db.disable_torrent_seeding(id).await?;
        if self.torrents.has_task(id).await {
            if let Some((down, up, size)) = self.torrents.transfer_snapshot(id).await {
                self.db.observe_transfer(id, down, up, size).await?;
            }
            self.torrents.forget_task(id).await?;
            if !self.lifecycle.get().await.retain_after_use() {
                self.torrents.cool_if_idle().await;
            }
        }
        self.db.get(id).await?.context("task not found")
    }
    pub async fn select_torrent_files(&self, id: &str, indices: &[usize]) -> Result<serde_json::Value> {
        let task = self.db.get(id).await?.context("task not found")?;
        if task.engine != "librqbit" || !matches!(task.status.as_str(), "running" | "paused") {
            bail!("file selection requires an active or paused torrent");
        }
        self.torrents.select_files(id, indices).await?;
        self.db.save_torrent_selection(id, indices).await?;
        self.torrent_details(id).await
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

    /// Restart a failed/interrupted job using the same task ID and download location.
    /// Running, queued, paused, and completed tasks are never retried by this endpoint.
    pub async fn retry(&self, id: &str) -> Result<Download> {
        let task = self.db.requeue_failed(id).await?
            .context("task is not failed/interrupted or does not exist")?;
        self.enqueue(task.clone());
        Ok(task)
    }

    /// Re-attach completed BitTorrent jobs from SQLite on daemon startup.
    /// Failures are logged but never change completed tasks into failures.
    /// The shared semaphore caps concurrent disk rechecks and metadata resolution.
    pub async fn restore_completed_torrents(&self) -> Result<()> {
        if !self.lifecycle.get().await.retain_after_use() { return Ok(()); }
        for item in self.db.completed_torrents_to_seed().await? {
            if self.torrents.has_task(&item.id).await { continue; }
            {
                let mut restoring = self.restoring_seeds.lock().await;
                if !restoring.insert(item.id.clone()) { continue; }
            }
            let this = self.clone();
            tokio::spawn(async move {
                let id = item.id.clone();
                let result = async {
                    let _permit = this.slots.acquire().await;
                    if !this.db.torrent_preferences(&id).await?.is_some_and(|p| p.seed_enabled) {
                        return Ok(());
                    }
                    if this.torrents.has_task(&id).await { return Ok(()); }
                    this.run_torrent(item).await
                }.await;
                if let Err(error) = result {
                    tracing::warn!(task_id=%id, "cannot reattach completed torrent: {error:#}");
                }
                this.restoring_seeds.lock().await.remove(&id);
            });
        }
        Ok(())
    }
    /// Start a bounded number of workers; every queued task is fetched
    /// directly from the persistent database. Can be called multiple times.
    pub async fn enqueue_pending(&self) -> Result<()> {
        if !self.queue_started.swap(true, Ordering::AcqRel) {
            // Bounded 32 Tokio dispatcher tasks regardless of backlog size.
            // LiveSlots gates the effective number of active transfers.
            for _ in 0..32 {
                let this = self.clone();
                tokio::spawn(async move { this.queue_worker_loop().await; });
            }
        }
        self.queue_notify.notify_waiters();
        Ok(())
    }
    async fn run_torrent(&self, task: Download) -> Result<()> {
        let preferences = self.db.torrent_preferences(&task.id).await?
            .context("missing torrent database task")?;
        if task.status == "completed" && !preferences.seed_enabled {
            return Ok(());
        }
        // A new librqbit handle has a fresh upload counter. Preserve the
        // database's previous upload total when retrying or reattaching seeds.
        let upload_offset = if task.attempts > 1 || task.status == "completed" {
            u64::try_from(task.uploaded_bytes.max(0))?
        } else { 0 };
        let resume = task.attempts > 1 || task.status == "completed";
        let handle = self.torrents.add(
            &task.id, &task.source, preferences.only_files, resume
        ).await?;
        // Stop-seeding may have been requested while magnet metadata resolved.
        if task.status == "completed" && !self.db.torrent_preferences(&task.id).await?
            .is_some_and(|p| p.seed_enabled) {
            self.torrents.forget_task(&task.id).await?;
            return Ok(());
        }
        let mut ticks = tokio::time::interval(Duration::from_secs(2));
        let completed = handle.wait_until_completed();
        tokio::pin!(completed);
        loop {
            tokio::select! {
                result = &mut completed => {
                    result?;
                    let stats = handle.stats();
                    self.db.observe_transfer(&task.id, stats.progress_bytes,
                        upload_offset.saturating_add(stats.uploaded_bytes), stats.total_bytes).await?;
                    self.db.finish(&task.id, true, None).await?;
                    break;
                }
                _ = ticks.tick() => {
                    let stats = handle.stats();
                    self.db.observe_transfer(&task.id, stats.progress_bytes,
                        upload_offset.saturating_add(stats.uploaded_bytes), stats.total_bytes).await?;
                }
            }
        }
        // An active torrent could have been removed or its seed intent disabled
        // while finishing the recheck; do not launch a second seed monitor.
        if self.db.get(&task.id).await?.is_some_and(|record| record.status != "completed") {
            return Ok(());
        }
        if !self.torrents.has_task(&task.id).await { return Ok(()); }
        if !self.db.torrent_preferences(&task.id).await?.is_some_and(|p| p.seed_enabled) {
            self.torrents.forget_task(&task.id).await?;
            return Ok(());
        }
        if !self.lifecycle.get().await.retain_after_use() {
            self.torrents.finish_temporary(&task.id).await?;
            return Ok(());
        }
        // Keep seeding after download; count new uploads on top of the
        // last value persisted by an earlier Kordra process.
        let db = self.db.clone();
        let id = task.id.clone();
        let seeding_handle = handle.clone();
        let torrents = self.torrents.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if !torrents.has_task(&id).await { break; }
                let stats = seeding_handle.stats();
                if let Err(error) = db.observe_transfer(&id, stats.progress_bytes,
                    upload_offset.saturating_add(stats.uploaded_bytes), stats.total_bytes).await {
                    tracing::warn!(task_id=%id, "seeding stats unavailable: {error:#}");
                    break;
                }
            }
        });
        Ok(())
    }
    async fn run_ytdlp(&self, task: Download, cancellation: CancellationToken) -> Result<()> {
        // yt-dlp has no persistent RPC control. New tasks take a snapshot of
        // the current limit; running CLI processes keep their existing rate.
        let download_limit = self.transfer.get().await.yt_dlp_download_bps;
        let binary = tokio::select! {
            result = self.manager.ensure_active() => result?,
            _ = cancellation.cancelled() => {
                self.db.finish_cancelled(&task.id).await?;
                return Ok(());
            }
        };
        tokio::fs::create_dir_all(&self.downloads).await?;
        let mut command = Command::new(binary);
        if let Some(limit) = download_limit {
            command.arg("--limit-rate").arg(limit.to_string());
        }
        if youtube_url(&task.source) {
            let deno = tokio::select! {
                result = self.deno.ensure_active() => result.context("installing Deno required for YouTube")?,
                _ = cancellation.cancelled() => {
                    self.db.finish_cancelled(&task.id).await?;
                    return Ok(());
                }
            };
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
        let status = tokio::select! {
            result = child.wait() => Some(result?),
            _ = cancellation.cancelled() => {
                // Stop and reap the direct child; grandchildren require dedicated
                // process-tree supervision (Windows Job Object/Unix process group).
                let _ = child.kill().await;
                None
            }
        };
        let _ = tokio::join!(err_reader, out_reader);
        if status.is_none() {
            self.db.finish_cancelled(&task.id).await?;
            return Ok(());
        }
        let status = status.context("missing yt-dlp exit status")?;
        if status.success() {
            self.db.finish(&task.id, true, None).await?;
            // A concurrent cancellation request can win just before completion.
            self.db.finish_cancelled(&task.id).await?;
        } else if !self.db.finish_cancelled(&task.id).await? {
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
