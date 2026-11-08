//! Versioned, persisted engine activation policy.
//! Embedded BitTorrent has a long-lived session; yt-dlp/Deno are CLI tools.
//! Persistent CLI mode holds an adapter ready in Kordra, not a fake idle subprocess.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupMode {
    Hot,
    #[default]
    Cold,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdRetention {
    Temporary,
    #[default]
    Persistent,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineLifecyclePolicy {
    pub startup: StartupMode,
    pub cold_retention: ColdRetention,
}
impl EngineLifecyclePolicy {
    pub fn retain_after_use(self) -> bool {
        self.startup == StartupMode::Hot || self.cold_retention == ColdRetention::Persistent
    }
}
#[derive(Clone)]
pub struct PolicyStore {
    path: PathBuf,
    policy: Arc<RwLock<EngineLifecyclePolicy>>,
    writes: Arc<Mutex<()>>,
}
impl PolicyStore {
    pub async fn open(data: PathBuf) -> Result<Self> {
        let path = data.join("config").join("engine-lifecycle.json");
        let policy = match tokio::fs::read(&path).await {
            Ok(contents) => serde_json::from_slice(&contents)
                .context("invalid engine lifecycle configuration")?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound =>
                EngineLifecyclePolicy::default(),
            Err(err) => return Err(err.into()),
        };
        Ok(Self { path, policy: Arc::new(RwLock::new(policy)), writes: Arc::new(Mutex::new(())) })
    }
    pub async fn get(&self) -> EngineLifecyclePolicy { *self.policy.read().await }
    pub async fn update(&self, next: EngineLifecyclePolicy) -> Result<()> {
        let _guard = self.writes.lock().await;
        let dir = self.path.parent().context("configuration directory missing")?;
        tokio::fs::create_dir_all(dir).await?;
        let staged = self.path.with_extension("json.partial");
        tokio::fs::write(&staged, serde_json::to_vec_pretty(&next)?).await?;
        // On Windows, rename cannot replace an existing target. Rename the old
        // configuration to a backup so failure does not silently erase settings.
        if tokio::fs::try_exists(&self.path).await? {
            let backup = self.path.with_extension("json.previous");
            if tokio::fs::try_exists(&backup).await? { tokio::fs::remove_file(&backup).await?; }
            tokio::fs::rename(&self.path, &backup).await?;
            if let Err(error) = tokio::fs::rename(&staged, &self.path).await {
                let _ = tokio::fs::rename(&backup, &self.path).await;
                return Err(error.into());
            }
            let _ = tokio::fs::remove_file(backup).await;
        } else {
            tokio::fs::rename(&staged, &self.path).await?;
        }
        *self.policy.write().await = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_modes_and_effective_retention() {
        let hot = EngineLifecyclePolicy { startup: StartupMode::Hot, cold_retention: ColdRetention::Temporary };
        assert!(hot.retain_after_use());
        let cold = EngineLifecyclePolicy::default();
        assert_eq!(cold.startup, StartupMode::Cold);
        assert_eq!(cold.cold_retention, ColdRetention::Persistent);
        assert!(cold.retain_after_use());
        let short = EngineLifecyclePolicy { startup: StartupMode::Cold, cold_retention: ColdRetention::Temporary };
        assert!(!short.retain_after_use());
        let invalid = serde_json::from_str::<EngineLifecyclePolicy>(r#"{"startup":"warm","cold_retention":"temporary"}"#);
        assert!(invalid.is_err());
    }
    #[tokio::test]
    async fn round_trip_persists() {
        let p = std::env::temp_dir().join(format!("kordra-lifecycle-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&p).await;
        let store = PolicyStore::open(p.clone()).await.unwrap();
        let policy = EngineLifecyclePolicy { startup: StartupMode::Hot, cold_retention: ColdRetention::Persistent };
        store.update(policy).await.unwrap();
        assert_eq!(PolicyStore::open(p.clone()).await.unwrap().get().await, policy);
        store.update(EngineLifecyclePolicy::default()).await.unwrap();
        assert_eq!(PolicyStore::open(p.clone()).await.unwrap().get().await, EngineLifecyclePolicy::default());
        let _ = tokio::fs::remove_dir_all(p).await;
    }
}
