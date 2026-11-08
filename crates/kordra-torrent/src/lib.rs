//! Embedded default BitTorrent engine. One lazily initialized rqbit session per Kordra process.
//! Remote authentication, file priority controls and persistence integration come next.
use anyhow::{bail, Context, Result};
use librqbit::{AddTorrent, AddTorrentResponse, ManagedTorrent, Session};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::{OnceCell, RwLock};

#[derive(Clone)]
pub struct TorrentEngine {
    output: PathBuf,
    session: Arc<OnceCell<Arc<Session>>>,
    active: Arc<RwLock<HashMap<String, Arc<ManagedTorrent>>>>,
}

impl TorrentEngine {
    pub fn new(output: PathBuf) -> Self {
        Self { output, session: Arc::new(OnceCell::new()), active: Arc::new(RwLock::new(HashMap::new())) }
    }

    async fn session(&self) -> Result<&Arc<Session>> {
        self.session.get_or_try_init(|| async {
            tokio::fs::create_dir_all(&self.output).await?;
            let session = Session::new(self.output.clone()).await?;
            Ok(session)
        }).await
    }

    pub async fn add(&self, task_id: &str, source: &str) -> Result<Arc<ManagedTorrent>> {
        if !(source.starts_with("magnet:?") || source.starts_with("https://") || source.starts_with("http://")) {
            bail!("only magnet links and HTTP(S) .torrent links are accepted");
        }
        let result = self.session().await?.add_torrent(AddTorrent::from_url(source), None).await?;
        match result {
            AddTorrentResponse::Added(_, handle) => {
                self.active.write().await.insert(task_id.to_string(), handle.clone());
                Ok(handle)
            },
            AddTorrentResponse::AlreadyManaged(_, _) => bail!("torrent is already managed by Kordra"),
            AddTorrentResponse::ListOnly(_) => bail!("unexpected list-only result"),
        }
    }

    /// Live diagnostic snapshot; never starts a torrent or reads arbitrary filesystem paths.
    pub async fn details(&self, task_id: &str) -> Option<Value> {
        let handle = self.active.read().await.get(task_id).cloned()?;
        let stats = handle.stats();
        let trackers: Vec<String> = handle.shared().trackers.iter().map(ToString::to_string).collect();
        Some(json!({
            "id": handle.id(),
            "name": handle.name(),
            "info_hash": handle.info_hash().as_string(),
            "paused": handle.is_paused(),
            "output_folder": handle.output_folder().to_string_lossy(),
            "only_files": handle.only_files(),
            "trackers": trackers,
            "stats": stats
        }))
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
    fn torrent_module_is_available() {
        let manager = super::TorrentEngine::new(std::env::temp_dir().join("kordra-test-torrents"));
        let _cloned = manager.clone();
        // Test does not initialize DHT/listeners; CI integration tests follow.
    }
}
