//! Versioned Deno runtime with verified release ZIP assets and fallback.
use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::{Cursor, Read}, path::{Path, PathBuf}, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DenoState {
    pub active: Option<String>,
    pub fallback: Option<String>,
    pub pinned: Option<String>,
    pub installed: Vec<String>,
    pub auto_update: bool,
}
#[derive(Clone)]
pub struct DenoManager {
    root: PathBuf,
    client: Client,
    lock: Arc<Mutex<()>>,
}
#[derive(Deserialize)]
struct Release { tag_name: String, assets: Vec<Asset> }
#[derive(Deserialize)]
struct Asset { name: String, browser_download_url: String }

fn valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() < 80 && tag != "." && tag != ".."
        && tag.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}
fn asset_name() -> Result<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))] { return Ok("deno-x86_64-pc-windows-msvc.zip"); }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))] { return Ok("deno-aarch64-pc-windows-msvc.zip"); }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))] { return Ok("deno-x86_64-unknown-linux-gnu.zip"); }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))] { return Ok("deno-aarch64-unknown-linux-gnu.zip"); }
    #[allow(unreachable_code)]
    bail!("Deno platform asset is not available for this target")
}
fn executable() -> &'static str { if cfg!(windows) { "deno.exe" } else { "deno" } }
fn checksum(text: &str) -> Result<String> {
    let hash = text.split_whitespace().next().context("empty checksum")?;
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid Deno checksum");
    }
    Ok(hash.to_ascii_lowercase())
}
fn extract_binary(bytes: Vec<u8>) -> Result<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut entry = archive.by_name(executable()).context("Deno binary missing in verified release archive")?;
    if entry.size() > 250_000_000 { bail!("Deno binary is too large"); }
    let mut output = Vec::new();
    entry.take(250_000_001).read_to_end(&mut output)?;
    if output.len() > 250_000_000 || output.is_empty() { bail!("invalid Deno executable size"); }
    Ok(output)
}

impl DenoManager {
    pub fn new(data: PathBuf) -> Result<Self> {
        Ok(Self {
            root: data.join("runtimes").join("deno"),
            client: Client::builder()
                .user_agent("Kordra/0.1 (+https://github.com/KordraProject/Kordra)")
                .timeout(Duration::from_secs(180)).build()?,
            lock: Arc::new(Mutex::new(())),
        })
    }
    fn path(&self, tag: &str) -> Result<PathBuf> {
        if !valid_tag(tag) { bail!("invalid release tag"); }
        Ok(self.root.join("versions").join(tag).join(executable()))
    }
    async fn read(&self) -> Result<DenoState> {
        match tokio::fs::read(self.root.join("state.json")).await {
            Ok(v) => Ok(serde_json::from_slice(&v)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound =>
                Ok(DenoState { auto_update: true, ..Default::default() }),
            Err(e) => Err(e.into()),
        }
    }
    async fn write(&self, value: &DenoState) -> Result<()> {
        tokio::fs::create_dir_all(&self.root).await?;
        let temp = self.root.join("state.json.partial");
        tokio::fs::write(&temp, serde_json::to_vec_pretty(value)?).await?;
        #[cfg(windows)]
        if tokio::fs::metadata(self.root.join("state.json")).await.is_ok() {
            tokio::fs::remove_file(self.root.join("state.json")).await?;
        }
        tokio::fs::rename(temp, self.root.join("state.json")).await?;
        Ok(())
    }
    pub async fn state(&self) -> Result<DenoState> {
        let _guard = self.lock.lock().await;
        self.read().await
    }
    async fn check(path: &Path) -> Result<()> {
        let result = tokio::time::timeout(Duration::from_secs(20),
            tokio::process::Command::new(path).arg("--version").kill_on_drop(true).output()).await??;
        if !result.status.success() { bail!("Deno health check failed"); }
        Ok(())
    }
    pub async fn active_path(&self) -> Result<Option<PathBuf>> {
        let _guard = self.lock.lock().await;
        match self.read().await?.active {
            Some(version) => {
                let path = self.path(&version)?;
                if !path.is_file() { bail!("selected Deno version is missing"); }
                Ok(Some(path))
            }
            None => Ok(None),
        }
    }
    async fn release(&self, tag: Option<&str>) -> Result<Release> {
        let url = match tag {
            None => "https://api.github.com/repos/denoland/deno/releases/latest".to_string(),
            Some(tag) if valid_tag(tag) => format!("https://api.github.com/repos/denoland/deno/releases/tags/{tag}"),
            _ => bail!("invalid version"),
        };
        Ok(self.client.get(url).send().await?.error_for_status()?.json().await?)
    }
    async fn install_release(&self, release: Release, activate: bool) -> Result<DenoState> {
        if !valid_tag(&release.tag_name) { bail!("invalid upstream tag"); }
        let mut state = self.read().await?;
        let archive = asset_name()?;
        let tag = release.tag_name;
        let path = self.path(&tag)?;
        if !path.is_file() {
            let zip_asset = release.assets.iter().find(|a| a.name == archive)
                .context("no Deno ZIP asset for this platform")?;
            let sum_asset = release.assets.iter().find(|a| a.name == format!("{archive}.sha256sum"))
                .context("no matching SHA-256 checksum asset")?;
            let expected = checksum(&self.client.get(&sum_asset.browser_download_url)
                .send().await?.error_for_status()?.text().await?)?;
            let response = self.client.get(&zip_asset.browser_download_url)
                .send().await?.error_for_status()?;
            if response.content_length().is_some_and(|x| x > 125_000_000) { bail!("ZIP too large"); }
            let bytes = response.bytes().await?;
            if bytes.len() > 125_000_000 { bail!("ZIP too large"); }
            if hex::encode(Sha256::digest(&bytes)) != expected { bail!("Deno checksum mismatch"); }
            let unzipped = tokio::task::spawn_blocking(move || extract_binary(bytes.to_vec())).await??;
            let directory = path.parent().context("bad tool path")?;
            tokio::fs::create_dir_all(directory).await?;
            let stage = directory.join(if cfg!(windows) { "deno.partial.exe" } else { "deno.partial" });
            tokio::fs::write(&stage, &unzipped).await?;
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o700)).await?;
            }
            Self::check(&stage).await?;
            tokio::fs::rename(stage, &path).await?;
        } else { Self::check(&path).await?; }
        if !state.installed.contains(&tag) { state.installed.push(tag.clone()); }
        if activate && state.pinned.is_none() && state.active.as_deref() != Some(&tag) {
            state.fallback = state.active.take();
            state.active = Some(tag);
        }
        self.write(&state).await?;
        Ok(state)
    }
    pub async fn update(&self, activate: bool) -> Result<DenoState> {
        let _guard = self.lock.lock().await;
        let release = self.release(None).await?;
        self.install_release(release, activate).await
    }
    pub async fn install_version(&self, tag: &str, activate: bool) -> Result<DenoState> {
        let _guard = self.lock.lock().await;
        let release = self.release(Some(tag)).await?;
        self.install_release(release, activate).await
    }
    pub async fn activate(&self, tag: &str) -> Result<DenoState> {
        let _guard = self.lock.lock().await;
        let mut state = self.read().await?;
        if !state.installed.iter().any(|v| v == tag) { bail!("Deno version is not installed"); }
        Self::check(&self.path(tag)?).await?;
        if state.active.as_deref() != Some(tag) {
            state.fallback = state.active.take();
            state.active = Some(tag.to_string());
        }
        self.write(&state).await?;
        Ok(state)
    }
    pub async fn rollback(&self) -> Result<DenoState> {
        let fallback = self.state().await?.fallback.context("no Deno fallback available")?;
        self.activate(&fallback).await
    }
    pub async fn policy(&self, pinned: Option<String>, auto_update: bool) -> Result<DenoState> {
        let _guard = self.lock.lock().await;
        let mut state = self.read().await?;
        if let Some(version) = pinned.as_ref() {
            if !state.installed.contains(version) { bail!("Deno pin must refer to an installed version"); }
            if state.active.as_ref() != Some(version) {
                state.fallback = state.active.take();
                state.active = Some(version.clone());
            }
        }
        state.pinned = pinned;
        state.auto_update = auto_update;
        self.write(&state).await?;
        Ok(state)
    }
    pub async fn ensure_active(&self) -> Result<PathBuf> {
        if let Some(path) = self.active_path().await? { return Ok(path); }
        let state = self.state().await?;
        if state.pinned.is_some() { bail!("pinned Deno binary is unavailable"); }
        let updated = self.update(true).await?;
        self.path(updated.active.as_deref().context("Deno installation did not activate")?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_unsafe_tags_and_bad_checksums() {
        assert!(!valid_tag("../bad"));
        assert!(valid_tag("v2.9.8"));
        assert!(checksum("invalid").is_err());
        assert_eq!(checksum(&"a".repeat(64)).unwrap(), "a".repeat(64));
    }
    #[test]
    fn reject_non_zip_data() {
        assert!(extract_binary(b"not a zip archive".to_vec()).is_err());
    }
}
