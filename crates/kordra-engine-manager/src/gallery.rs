//! Standalone gallery-dl rolling builds from the upstream gdl-org/builds repository.
//! Install only assets with a matching GitHub SHA-256 digest. No arbitrary URLs.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{io::AsyncWriteExt, sync::Mutex};

const LIMIT: u64 = 100_000_000;
#[derive(Clone)]
pub struct GalleryManager {
    root: PathBuf,
    client: reqwest::Client,
    lock: Arc<Mutex<()>>,
}
#[derive(Deserialize)]
struct Release { tag_name: String, assets: Vec<Asset> }
#[derive(Deserialize)]
struct Asset { name: String, size: u64, digest: Option<String>, browser_download_url: String }
fn valid_tag(tag: &str) -> bool {
    let b = tag.as_bytes();
    b.len() == 10 && b[4] == b'.' && b[7] == b'.' &&
        b.iter().enumerate().all(|(i,c)| i==4 || i==7 || c.is_ascii_digit())
}
fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows","x86_64") => Some("gallery-dl_windows.exe"),
        ("linux","x86_64") => Some("gallery-dl_linux"),
        _ => None,
    }
}
fn executable() -> &'static str { if cfg!(windows) { "gallery-dl.exe" } else { "gallery-dl" } }
fn verified_asset(release: &Release, name: &str) -> Result<(String, u64, String)> {
    anyhow::ensure!(valid_tag(&release.tag_name), "invalid gallery-dl build tag");
    let asset = release.assets.iter().find(|asset| asset.name == name).context("no supported gallery-dl release asset")?;
    anyhow::ensure!((1..=LIMIT).contains(&asset.size), "invalid gallery-dl asset size");
    let url = format!("https://github.com/gdl-org/builds/releases/download/{}/{}", release.tag_name, name);
    anyhow::ensure!(asset.browser_download_url == url, "unexpected upstream asset URL");
    let digest = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).context("upstream did not publish a SHA-256 digest")?;
    anyhow::ensure!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()), "invalid upstream SHA-256");
    Ok((url, asset.size, digest.to_ascii_lowercase()))
}
impl GalleryManager {
    pub fn new(data: PathBuf) -> Result<Self> {
        Ok(Self { root: data.join("engines").join("gallery-dl").join("versions"),
            client: reqwest::Client::builder().user_agent("Kordra/0.1").timeout(Duration::from_secs(180)).build()?,
            lock: Arc::new(Mutex::new(())) })
    }
    pub fn supported(&self) -> bool { asset_name().is_some() }
    pub async fn installed_version(&self) -> Result<Option<String>> {
        let mut entries = match tokio::fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut latest: Option<String> = None;
        while let Some(entry) = entries.next_entry().await? {
            let tag = entry.file_name().to_string_lossy().into_owned();
            if valid_tag(&tag) && entry.file_type().await?.is_dir() &&
                entry.path().join(executable()).is_file() &&
                latest.as_ref().is_none_or(|old| tag > *old) {
                latest = Some(tag);
            }
        }
        Ok(latest)
    }
    pub async fn active_path(&self) -> Result<Option<PathBuf>> {
        if let Some(tag) = self.installed_version().await? {
            return Ok(Some(tokio::fs::canonicalize(self.root.join(tag).join(executable())).await?));
        }
        Ok(None)
    }
    pub async fn ensure_active(&self) -> Result<PathBuf> {
        let _guard = self.lock.lock().await;
        if let Some(path) = self.active_path().await? { return Ok(path); }
        self.install_latest().await
    }
    pub async fn update(&self) -> Result<PathBuf> {
        let _guard = self.lock.lock().await;
        self.install_latest().await
    }
    async fn install_latest(&self) -> Result<PathBuf> {
        let name = asset_name().context("managed gallery-dl unavailable on this platform; install a local executable")?;
        let release: Release = self.client.get("https://api.github.com/repos/gdl-org/builds/releases/latest")
            .send().await?.error_for_status()?.json().await?;
        let (url, size, expected) = verified_asset(&release,name)?;
        let directory = self.root.join(&release.tag_name);
        let path = directory.join(executable());
        if path.is_file() { return Ok(tokio::fs::canonicalize(path).await?); }
        tokio::fs::create_dir_all(&directory).await?;
        let stage = directory.join(if cfg!(windows) { "gallery-dl.partial.exe" } else { "gallery-dl.partial" });
        let result = async {
            let mut response = self.client.get(url).send().await?.error_for_status()?;
            if response.content_length().is_some_and(|n| n != size) { bail!("asset size differs from release metadata"); }
            let mut file = tokio::fs::File::create(&stage).await?;
            let mut hasher = Sha256::new();
            let mut count = 0u64;
            while let Some(chunk) = response.chunk().await? {
                count = count.saturating_add(chunk.len() as u64);
                anyhow::ensure!(count <= size && count <= LIMIT, "gallery-dl download exceeded verified size");
                hasher.update(&chunk);
                file.write_all(&chunk).await?;
            }
            anyhow::ensure!(count == size && hex::encode(hasher.finalize()) == expected, "gallery-dl checksum mismatch");
            file.sync_all().await?;
            drop(file);
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o700)).await?;
            }
            let health = tokio::time::timeout(Duration::from_secs(30),
                tokio::process::Command::new(tokio::fs::canonicalize(&stage).await?)
                    .arg("--version").stdin(std::process::Stdio::null()).kill_on_drop(true).output()).await??;
            anyhow::ensure!(health.status.success(), "gallery-dl executable failed its version check");
            tokio::fs::rename(&stage,&path).await?;
            Ok::<PathBuf,anyhow::Error>(tokio::fs::canonicalize(&path).await?)
        }.await;
        if result.is_err() { let _ = tokio::fs::remove_file(&stage).await; }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asset_validation_rejects_missing_digests_and_untrusted_paths() {
        let mut r = Release { tag_name:"2026.11.27".into(),assets:vec![Asset{
            name:"gallery-dl_linux".into(), size:12, digest:Some(format!("sha256:{}", "a".repeat(64))),
            browser_download_url:"https://github.com/gdl-org/builds/releases/download/2026.11.27/gallery-dl_linux".into()
        }] };
        assert!(verified_asset(&r,"gallery-dl_linux").is_ok());
        r.assets[0].digest=None;
        assert!(verified_asset(&r,"gallery-dl_linux").is_err());
        r.assets[0].digest=Some(format!("sha256:{}", "a".repeat(64)));
        r.assets[0].browser_download_url="https://example.org/untrusted".into();
        assert!(verified_asset(&r,"gallery-dl_linux").is_err());
        r.tag_name="../bad".into();
        assert!(verified_asset(&r,"gallery-dl_linux").is_err());
    }
}
