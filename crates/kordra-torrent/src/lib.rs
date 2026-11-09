//! Embedded BitTorrent session with hot/cold startup and idle teardown.
use anyhow::{bail, Context, Result};
use librqbit::{api::TorrentIdOrHash, AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent, Session};
use serde_json::{json, Value};
use std::{collections::{HashMap, HashSet}, num::NonZeroU32, path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, RwLock};

#[derive(Clone)]
pub struct TorrentEngine {
    output: PathBuf,
    session: Arc<Mutex<Option<Arc<Session>>>>,
    operations: Arc<Mutex<()>>,
    active: Arc<RwLock<HashMap<String, Arc<ManagedTorrent>>>>,
    rate_settings: Arc<RwLock<(Option<u32>, Option<u32>)>>,
}
impl TorrentEngine {
    pub fn new(output: PathBuf) -> Self {
        Self { output, session: Arc::new(Mutex::new(None)),
            operations: Arc::new(Mutex::new(())),
            active: Arc::new(RwLock::new(HashMap::new())),
            rate_settings: Arc::new(RwLock::new((None, None))) }
    }
    async fn session(&self) -> Result<Arc<Session>> {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_ref() { return Ok(session.clone()); }
        tokio::fs::create_dir_all(&self.output).await?;
        let instance = Session::new(self.output.clone()).await?;
        let (download, upload) = *self.rate_settings.read().await;
        instance.ratelimits.set_download_bps(download.and_then(NonZeroU32::new));
        instance.ratelimits.set_upload_bps(upload.and_then(NonZeroU32::new));
        *guard = Some(instance.clone());
        Ok(instance)
    }
    /// Start BitTorrent listeners without downloading a torrent.
    pub async fn warm(&self) -> Result<()> {
        let _operation = self.operations.lock().await;
        let _ = self.session().await?;
        Ok(())
    }
    /// Apply global BitTorrent bandwidth limits to an already running session
    /// and to any new session created in cold mode. 0/None means unlimited.
    pub async fn set_rate_limits(&self, download_bps: Option<u32>, upload_bps: Option<u32>) {
        *self.rate_settings.write().await = (download_bps, upload_bps);
        if let Some(session) = self.session.lock().await.as_ref() {
            session.ratelimits.set_download_bps(download_bps.and_then(NonZeroU32::new));
            session.ratelimits.set_upload_bps(upload_bps.and_then(NonZeroU32::new));
        }
    }
    pub async fn rate_limits(&self) -> (Option<u32>, Option<u32>) {
        *self.rate_settings.read().await
    }
    pub async fn is_ready(&self) -> bool { self.session.lock().await.is_some() }
    pub async fn active_count(&self) -> usize { self.active.read().await.len() }

    pub async fn add(&self, task_id: &str, source: &str, only_files: Option<Vec<usize>>, resume: bool) -> Result<Arc<ManagedTorrent>> {
        if !(source.starts_with("magnet:?") || source.starts_with("https://") || source.starts_with("http://")) {
            bail!("only magnet links and HTTP(S) .torrent links are accepted");
        }
        // Serialize add/idle-stop so no new torrent is attached to a closing session.
        let _operation = self.operations.lock().await;
        let session = self.session().await?;
        // Resuming against an existing file requires overwrite=true in librqbit.
        // This reuses the same output root and lets librqbit re-check the data.
        let opts = AddTorrentOptions { overwrite: resume, only_files, ..Default::default() };
        let result = session.add_torrent(AddTorrent::from_url(source), Some(opts)).await?;
        match result {
            AddTorrentResponse::Added(_, handle) => {
                self.active.write().await.insert(task_id.to_string(), handle.clone());
                Ok(handle)
            }
            AddTorrentResponse::AlreadyManaged(_, _) => bail!("torrent is already managed by Kordra"),
            AddTorrentResponse::ListOnly(_) => bail!("unexpected list-only result"),
        }
    }
    /// Temporary cold mode: forget one completed torrent without removing its files;
    /// stop all listeners after the final active task. Completed seeding is not retained.
    pub async fn finish_temporary(&self, task_id: &str) -> Result<()> {
        // Removing a handle from our map alone did not remove the torrent
        // from librqbit: it could continue uploading while another task ran.
        self.forget_task(task_id).await?;
        self.cool_if_idle().await;
        Ok(())
    }
    pub async fn cool_if_idle(&self) {
        let _operation = self.operations.lock().await;
        if self.active.read().await.is_empty() {
            if let Some(session) = self.session.lock().await.take() {
                session.stop().await;
            }
        }
    }
    pub async fn details(&self, task_id: &str) -> Option<Value> {
        let handle = self.active.read().await.get(task_id).cloned()?;
        let stats = handle.stats();
        let trackers: Vec<String> = handle.shared().trackers.iter().map(ToString::to_string).collect();
        let selected = handle.only_files();
        let files = handle.with_metadata(|meta| {
            meta.info.iter_file_details().enumerate().map(|(index, file)| {
                json!({
                    "index": index,
                    "name": file.filename.to_string(),
                    "length": file.len,
                    "included": selected.as_ref().is_none_or(|items| items.contains(&index)),
                    "downloaded_bytes": stats.file_progress.get(index).copied().unwrap_or(0)
                })
            }).collect::<Vec<Value>>()
        }).ok()?;
        Some(json!({
            "id": handle.id(), "name": handle.name(),
            "info_hash": handle.info_hash().as_string(), "paused": handle.is_paused(),
            "output_folder": handle.output_folder().to_string_lossy(),
            "only_files": selected, "trackers": trackers, "files": files, "stats": stats
        }))
    }
    /// Update the file inclusion set for an existing live torrent.
    pub async fn select_files(&self, task_id: &str, indices: &[usize]) -> Result<()> {
        if indices.is_empty() || indices.len() > 10_000 {
            bail!("select at least one file, at most 10000");
        }
        let handle = self.active.read().await.get(task_id).cloned()
            .context("torrent is not active in this session")?;
        let total = handle.with_metadata(|meta| meta.info.iter_file_details().count())?;
        if indices.iter().any(|&idx| idx >= total) {
            bail!("file index is outside torrent metadata");
        }
        let included = indices.iter().copied().collect::<HashSet<usize>>();
        self.session().await?.update_only_files(&handle, &included).await
            .context("updating torrent file selection")
    }
    /// Stop managing a torrent without touching files. Must serialize with add/stop.
    pub async fn forget_task(&self, task_id: &str) -> Result<()> {
        let _operation = self.operations.lock().await;
        let handle = self.active.read().await.get(task_id).cloned()
            .context("torrent is not active in this session")?;
        let session = self.session.lock().await.as_ref().cloned()
            .context("torrent session is not running")?;
        session.delete(TorrentIdOrHash::Id(handle.id()), false).await
            .context("forgetting torrent without removing downloaded files")?;
        self.active.write().await.remove(task_id);
        Ok(())
    }
    pub async fn transfer_snapshot(&self, task_id: &str) -> Option<(u64, u64, u64)> {
        let handle = self.active.read().await.get(task_id).cloned()?;
        let stats = handle.stats();
        Some((stats.progress_bytes, stats.uploaded_bytes, stats.total_bytes))
    }
    pub async fn has_task(&self, task_id: &str) -> bool {
        self.active.read().await.contains_key(task_id)
    }
    pub async fn pause_task(&self, task_id: &str) -> Result<()> {
        let handle = self.active.read().await.get(task_id).cloned()
            .context("torrent is still starting or unavailable")?;
        self.session().await?.pause(&handle).await.context("pausing torrent")
    }
    pub async fn resume_task(&self, task_id: &str) -> Result<()> {
        let handle = self.active.read().await.get(task_id).cloned()
            .context("torrent is not present in the current process")?;
        self.session().await?.unpause(&handle).await.context("resuming torrent")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn torrent_manager_does_not_start_listeners_on_construction() {
        let manager = super::TorrentEngine::new(std::env::temp_dir().join("kordra-test-torrents"));
        let _copy = manager.clone();
    }
}
