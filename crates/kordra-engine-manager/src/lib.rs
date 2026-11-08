mod deno;
pub use deno::{DenoManager, DenoState};

//! Safe, managed yt-dlp standalone releases. Deno/FFmpeg catalogs follow later.
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::{Path, PathBuf}, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Channel { Stable, Nightly, Master }
impl Channel {
    fn upstream(self) -> &'static str {
        match self {
            Self::Stable => "yt-dlp/yt-dlp",
            Self::Nightly => "yt-dlp/yt-dlp-nightly-builds",
            Self::Master => "yt-dlp/yt-dlp-master-builds",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionState {
    pub channel: Channel,
    pub active: Option<String>,
    pub fallback: Option<String>,
    pub pinned: Option<String>,
    pub auto_update: bool,
    pub installed: Vec<String>,
}
impl Default for VersionState {
    fn default() -> Self {
        Self { channel: Channel::Stable, active: None, fallback: None,
            pinned: None, auto_update: true, installed: vec![] }
    }
}
#[derive(Clone)]
pub struct YtDlpManager { root: PathBuf, client: Client, lock: Arc<Mutex<()>> }

#[derive(Deserialize)]
struct Release { tag_name: String, assets: Vec<Asset> }
#[derive(Deserialize)]
struct Asset { name: String, browser_download_url: String }

fn valid_version(version: &str) -> bool {
    version.len() <= 80 && !version.is_empty() && version != "." && version != ".." &&
        version.bytes().all(|c| c.is_ascii_alphanumeric() || [b'.', b'-', b'_'].contains(&c))
}
fn sha_for_asset(checksums: &str, asset: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let digest = fields.next()?;
        let filename = fields.next()?.trim_start_matches('*');
        (filename == asset && digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| digest.to_ascii_lowercase())
    })
}
fn binary_name() -> Result<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))] { return Ok("yt-dlp.exe"); }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))] { return Ok("yt-dlp_linux"); }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))] { return Ok("yt-dlp_linux_aarch64"); }
    #[allow(unreachable_code)]
    Err(anyhow!("yt-dlp standalone asset is not supported on this platform yet"))
}

impl YtDlpManager {
    pub fn new(root: PathBuf) -> Result<Self> {
        Ok(Self { root, lock: Arc::new(Mutex::new(())),
            client: Client::builder().user_agent("Kordra/0.1 (+https://github.com/KordraProject/Kordra)")
                .timeout(Duration::from_secs(180)).build()? })
    }
    fn root(&self) -> PathBuf { self.root.join("engines").join("yt-dlp") }
    fn path(&self, version: &str) -> Result<PathBuf> {
        if !valid_version(version) { bail!("invalid version"); }
        Ok(self.root().join("versions").join(version).join(binary_name()?))
    }
    async fn load(&self) -> Result<VersionState> {
        match tokio::fs::read(self.root().join("state.json")).await {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(VersionState::default()),
            Err(e) => Err(e.into()),
        }
    }
    async fn save(&self, state: &VersionState) -> Result<()> {
        tokio::fs::create_dir_all(self.root()).await?;
        let tmp = self.root().join("state.json.partial");
        tokio::fs::write(&tmp, serde_json::to_vec_pretty(state)?).await?;
        #[cfg(target_os = "windows")]
        if tokio::fs::metadata(self.root().join("state.json")).await.is_ok() {
            tokio::fs::remove_file(self.root().join("state.json")).await?;
        }
        tokio::fs::rename(tmp, self.root().join("state.json")).await?;
        Ok(())
    }
    pub async fn state(&self) -> Result<VersionState> {
        let _guard = self.lock.lock().await;
        self.load().await
    }
    pub async fn set_policy(&self, channel: Channel, pinned: Option<String>, auto_update: bool) -> Result<VersionState> {
        let _guard = self.lock.lock().await;
        let mut state = self.load().await?;
        if let Some(version) = &pinned {
            if !valid_version(version) || !state.installed.contains(version) {
                bail!("pinned version must already be installed");
            }
        }
        state.channel = channel;
        state.pinned = pinned.clone();
        state.auto_update = auto_update;
        if let Some(v) = pinned {
            if state.active.as_deref() != Some(&v) {
                state.fallback = state.active.take();
                state.active = Some(v);
            }
        }
        self.save(&state).await?;
        Ok(state)
    }
    pub async fn active_path(&self) -> Result<Option<PathBuf>> {
        let _guard = self.lock.lock().await;
        match self.load().await?.active {
            Some(version) => {
                let path = self.path(&version)?;
                if !path.is_file() { bail!("active yt-dlp executable is missing"); }
                Ok(Some(path))
            },
            None => Ok(None),
        }
    }
    async fn check_executable(&self, path: &Path) -> Result<()> {
        let output = tokio::time::timeout(Duration::from_secs(30),
            tokio::process::Command::new(path).arg("--version").kill_on_drop(true).output()).await??;
        if !output.status.success() { bail!("yt-dlp --version failed"); }
        Ok(())
    }
    pub async fn activate(&self, version: &str) -> Result<VersionState> {
        let _guard = self.lock.lock().await;
        let mut state = self.load().await?;
        if !state.installed.iter().any(|item| item == version) { bail!("version is not installed"); }
        self.check_executable(&self.path(version)?).await?;
        if state.active.as_deref() != Some(version) {
            state.fallback = state.active.take();
            state.active = Some(version.to_owned());
            self.save(&state).await?;
        }
        Ok(state)
    }
    pub async fn rollback(&self) -> Result<VersionState> {
        let fallback = self.state().await?.fallback.context("no fallback version available")?;
        self.activate(&fallback).await
    }
    async fn release(&self, channel: Channel, tag: Option<&str>) -> Result<Release> {
        let api = format!("https://api.github.com/repos/{}", channel.upstream());
        let url = match tag {
            Some(tag) => {
                if !valid_version(tag) { bail!("invalid tag"); }
                format!("{api}/releases/tags/{tag}")
            }
            None if channel == Channel::Stable => format!("{api}/releases/latest"),
            None => format!("{api}/releases?per_page=1"),
        };
        let response = self.client.get(url).send().await?.error_for_status()?;
        if tag.is_none() && channel != Channel::Stable {
            let releases: Vec<Release> = response.json().await?;
            releases.into_iter().next().context("no channel release available")
        } else {
            Ok(response.json().await?)
        }
    }
    async fn install_release(&self, release: Release, activate: bool, channel: Channel) -> Result<VersionState> {
        if !valid_version(&release.tag_name) { bail!("upstream release tag rejected"); }
        let version = release.tag_name;
        let mut state = self.load().await?;
        let asset = binary_name()?;
        let path = self.path(&version)?;
        if !path.is_file() {
            let binary = release.assets.iter().find(|a| a.name == asset).context("missing platform binary")?;
            let sums = release.assets.iter().find(|a| a.name == "SHA2-256SUMS").context("missing upstream checksum file")?;
            let checksum_data = self.client.get(&sums.browser_download_url).send().await?.error_for_status()?.text().await?;
            let expected = sha_for_asset(&checksum_data, asset).context("missing checksum")?;
            let response = self.client.get(&binary.browser_download_url).send().await?.error_for_status()?;
            if response.content_length().is_some_and(|n| n > 250_000_000) {
                bail!("binary size exceeds limit");
            }
            let bytes = response.bytes().await?;
            if bytes.len() > 250_000_000 { bail!("binary size exceeds limit"); }
            if hex::encode(Sha256::digest(&bytes)) != expected { bail!("SHA-256 mismatch"); }
            let dir = path.parent().context("invalid binary path")?;
            tokio::fs::create_dir_all(dir).await?;
            let staged = dir.join(if cfg!(windows) { format!("{asset}.partial.exe") } else { format!("{asset}.partial") });
            tokio::fs::write(&staged, &bytes).await?;
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o700)).await?;
            }
            // Incomplete installations stay unactivated.
            self.check_executable(&staged).await?;
            tokio::fs::rename(staged, &path).await?;
        } else {
            self.check_executable(&path).await?;
        }
        if !state.installed.contains(&version) { state.installed.push(version.clone()); }
        // Pinning prevents automatic activation. Explicit manual activate is separate.
        if activate && state.pinned.is_none() && state.active.as_deref() != Some(&version) {
            state.fallback = state.active.take();
            state.active = Some(version);
        }
        state.channel = channel;
        self.save(&state).await?;
        Ok(state)
    }
    pub async fn update(&self, channel: Channel, activate: bool) -> Result<VersionState> {
        let _guard = self.lock.lock().await;
        let release = self.release(channel, None).await?;
        self.install_release(release, activate, channel).await
    }
    pub async fn install_version(&self, channel: Channel, version: &str, activate: bool) -> Result<VersionState> {
        let _guard = self.lock.lock().await;
        let release = self.release(channel, Some(version)).await?;
        self.install_release(release, activate, channel).await
    }
    pub async fn ensure_active(&self) -> Result<PathBuf> {
        if let Some(path) = self.active_path().await? { return Ok(path); }
        let policy = self.state().await?;
        if let Some(pin) = policy.pinned {
            bail!("pinned yt-dlp version {pin} is not installed/active");
        }
        let state = self.update(policy.channel, true).await?;
        let version = state.active.context("installation did not activate yt-dlp")?;
        self.path(&version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocks_path_traversal() {
        assert!(!valid_version("../tmp"));
        assert!(!valid_version(""));
        assert!(valid_version("2026.10.01"));
    }
    #[test]
    fn checksum_file_parser() {
        let hash = "a".repeat(64);
        assert_eq!(sha_for_asset(&format!("{hash} *yt-dlp.exe"), "yt-dlp.exe"), Some(hash));
    }
}
