//! Embedded default BitTorrent engine. One lazily initialized rqbit session per Kordra process.
//! Remote authentication, file priority controls and persistence integration come next.
use anyhow::{bail, Context, Result};
use librqbit::{AddTorrent, AddTorrentResponse, ManagedTorrent, Session};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::OnceCell;

#[derive(Clone)]
pub struct TorrentEngine {
    output: PathBuf,
    session: Arc<OnceCell<Arc<Session>>>,
}

impl TorrentEngine {
    pub fn new(output: PathBuf) -> Self {
        Self { output, session: Arc::new(OnceCell::new()) }
    }

    async fn session(&self) -> Result<&Arc<Session>> {
        self.session.get_or_try_init(|| async {
            tokio::fs::create_dir_all(&self.output).await?;
            let session = Session::new(self.output.clone()).await?;
            Ok(session)
        }).await
    }

    pub async fn add(&self, source: &str) -> Result<Arc<ManagedTorrent>> {
        if !(source.starts_with("magnet:?") || source.starts_with("https://") || source.starts_with("http://")) {
            bail!("only magnet links and HTTP(S) .torrent links are accepted");
        }
        let result = self.session().await?.add_torrent(AddTorrent::from_url(source), None).await?;
        match result {
            AddTorrentResponse::Added(_, handle) => Ok(handle),
            AddTorrentResponse::AlreadyManaged(_, _) => bail!("torrent is already managed by Kordra"),
            AddTorrentResponse::ListOnly(_) => bail!("unexpected list-only result"),
        }
    }

    pub async fn pause(&self, handle: &Arc<ManagedTorrent>) -> Result<()> {
        self.session().await?.pause(handle).await.context("pausing torrent")
    }
    pub async fn resume(&self, handle: &Arc<ManagedTorrent>) -> Result<()> {
        self.session().await?.unpause(handle).await.context("resuming torrent")
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
