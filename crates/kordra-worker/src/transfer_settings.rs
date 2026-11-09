//! Live bounded worker slots and durable traffic limits.
//! Only claimed/running tasks occupy a slot; queued SQLite rows need no Tokio task.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicUsize, Ordering}}};
use tokio::sync::{Mutex, Notify, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferSettings {
    pub concurrency: usize,
    /// Effective for new yt-dlp processes (bytes per second).
    pub yt_dlp_download_bps: Option<u64>,
    /// Applied immediately to librqbit's session-wide limiter (bytes per second).
    pub torrent_download_bps: Option<u32>,
    pub torrent_upload_bps: Option<u32>,
}
impl TransferSettings {
    pub fn with_concurrency(concurrency: usize) -> Self {
        Self {
            concurrency, yt_dlp_download_bps: None,
            torrent_download_bps: None, torrent_upload_bps: None
        }
    }
    pub fn validate(self) -> Result<Self> {
        if !(1..=32).contains(&self.concurrency) {
            bail!("concurrency must be between 1 and 32");
        }
        if let Some(bps) = self.yt_dlp_download_bps {
            if !(1024..=1_000_000_000).contains(&bps) {
                bail!("yt_dlp_download_bps must be 1024..=1000000000 or null");
            }
        }
        // librqbit's quota limiter may request a complete block at once;
        // rates below 64 KiB/s can reject an individual block request.
        for (name, bps) in [
            ("torrent_download_bps", self.torrent_download_bps),
            ("torrent_upload_bps", self.torrent_upload_bps),
        ] {
            if let Some(value) = bps {
                if value < 65_536 { bail!("{name} must be >= 65536 or null"); }
            }
        }
        Ok(self)
    }
}

#[derive(Clone)]
pub struct TransferStore {
    path: PathBuf,
    settings: Arc<RwLock<TransferSettings>>,
    writes: Arc<Mutex<()>>,
}
impl TransferStore {
    pub async fn open(data: PathBuf, default_concurrency: usize) -> Result<Self> {
        let path = data.join("config").join("transfer-settings.json");
        let backup = path.with_extension("json.previous");
        // If the process was terminated between renaming the original to the
        // backup and installing the new file, restore the previous value.
        let raw = match tokio::fs::read(&path).await {
            Ok(value) => Some(value),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                match tokio::fs::read(&backup).await {
                    Ok(value) => Some(value),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
                    Err(err) => return Err(err.into()),
                }
            }
            Err(err) => return Err(err.into()),
        };
        let settings = if let Some(contents) = raw {
            serde_json::from_slice::<TransferSettings>(&contents)
                .context("invalid persisted transfer-settings.json")?.validate()?
        } else {
            TransferSettings::with_concurrency(default_concurrency).validate()?
        };
        Ok(Self {
            path, settings: Arc::new(RwLock::new(settings)), writes: Arc::new(Mutex::new(()))
        })
    }
    pub async fn get(&self) -> TransferSettings { *self.settings.read().await }
    pub async fn save(&self, settings: TransferSettings) -> Result<()> {
        let settings = settings.validate()?;
        let _guard = self.writes.lock().await;
        let parent = self.path.parent().context("config directory missing")?;
        tokio::fs::create_dir_all(parent).await?;
        let stage = self.path.with_extension("json.partial");
        let backup = self.path.with_extension("json.previous");
        tokio::fs::write(&stage, serde_json::to_vec_pretty(&settings)?).await?;
        if tokio::fs::try_exists(&self.path).await? {
            if tokio::fs::try_exists(&backup).await? {
                tokio::fs::remove_file(&backup).await?;
            }
            tokio::fs::rename(&self.path, &backup).await?;
            if let Err(error) = tokio::fs::rename(&stage, &self.path).await {
                let _ = tokio::fs::rename(&backup, &self.path).await;
                return Err(error.into());
            }
            let _ = tokio::fs::remove_file(backup).await;
        } else {
            tokio::fs::rename(stage, &self.path).await?;
        }
        *self.settings.write().await = settings;
        Ok(())
    }
}

/// A concurrency gate that can be resized without restarting or interrupting
/// active transfers. If the limit is lowered, currently active jobs complete,
/// and no new job is admitted until enough slots have been released.
#[derive(Clone)]
pub struct LiveSlots {
    inner: Arc<SlotInner>,
}
struct SlotInner {
    limit: AtomicUsize,
    active: AtomicUsize,
    changed: Notify,
}
pub struct SlotPermit {
    inner: Arc<SlotInner>,
}
impl Drop for SlotPermit {
    fn drop(&mut self) {
        self.inner.active.fetch_sub(1, Ordering::AcqRel);
        self.inner.changed.notify_waiters();
    }
}
impl LiveSlots {
    pub fn new(limit: usize) -> Self {
        assert!((1..=32).contains(&limit));
        Self { inner: Arc::new(SlotInner {
            limit: AtomicUsize::new(limit),
            active: AtomicUsize::new(0),
            changed: Notify::new()
        }) }
    }
    pub fn set_limit(&self, limit: usize) {
        assert!((1..=32).contains(&limit));
        self.inner.limit.store(limit, Ordering::Release);
        self.inner.changed.notify_waiters();
    }
    pub fn active(&self) -> usize { self.inner.active.load(Ordering::Acquire) }
    pub fn limit(&self) -> usize { self.inner.limit.load(Ordering::Acquire) }
    pub fn available(&self) -> usize { self.limit().saturating_sub(self.active()) }
    pub async fn acquire(&self) -> SlotPermit {
        loop {
            // Register before looking at the atomics: this avoids a lost
            // notification between checking capacity and waiting.
            let wake = self.inner.changed.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            let mut active = self.inner.active.load(Ordering::Acquire);
            while active < self.inner.limit.load(Ordering::Acquire) {
                match self.inner.active.compare_exchange_weak(
                    active, active + 1, Ordering::AcqRel, Ordering::Acquire
                ) {
                    Ok(_) => {
                        return SlotPermit { inner: self.inner.clone() };
                    }
                    Err(actual) => active = actual,
                }
            }
            wake.await;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn concurrency_shrink_is_non_preemptive_and_growth_unblocks() {
        use tokio::time::{timeout, Duration};
        let slots = LiveSlots::new(2);
        let a = slots.acquire().await;
        let b = slots.acquire().await;
        assert_eq!((slots.active(), slots.available()), (2, 0));
        slots.set_limit(1);
        let waiting = tokio::spawn({
            let slots = slots.clone();
            async move { slots.acquire().await }
        });
        drop(a);
        assert!(timeout(Duration::from_millis(70), async {
            while slots.active() != 2 { tokio::task::yield_now().await; }
        }).await.is_err());
        assert_eq!(slots.active(), 1);
        slots.set_limit(2);
        let c = timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap();
        assert_eq!(slots.active(), 2);
        drop(c);
        drop(b);
        assert_eq!(slots.active(), 0);
    }
    #[tokio::test]
    async fn validates_and_persists_limits() {
        let dir = std::env::temp_dir().join(format!("kordra-transfer-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let store = TransferStore::open(dir.clone(), 2).await.unwrap();
        let settings = TransferSettings {
            concurrency: 7, yt_dlp_download_bps: Some(524288),
            torrent_download_bps: Some(65536), torrent_upload_bps: None
        };
        store.save(settings).await.unwrap();
        assert_eq!(TransferStore::open(dir.clone(), 1).await.unwrap().get().await, settings);
        assert!(store.save(TransferSettings { concurrency: 0, ..settings }).await.is_err());
        assert!(store.save(TransferSettings { torrent_download_bps: Some(5), ..settings }).await.is_err());
        assert_eq!(store.get().await, settings);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
