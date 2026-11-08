use anyhow::{bail, Result};
use kordra_api::{router, ApiState};
use kordra_core::Database;
use kordra_engine_manager::{DenoManager, YtDlpManager};
use kordra_worker::Worker;
use std::{env, net::SocketAddr, path::PathBuf, time::Duration};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let address: SocketAddr = env::var("KORDRA_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8079".to_owned()).parse()?;
    let token = env::var("KORDRA_API_TOKEN").ok().filter(|v| !v.is_empty());
    let metrics_token = env::var("KORDRA_METRICS_TOKEN").ok().filter(|v| !v.is_empty());
    if !address.ip().is_loopback() && token.is_none() {
        bail!("non-loopback binding requires KORDRA_API_TOKEN (HTTPS not yet integrated; use private reverse proxy)");
    }
    let data = PathBuf::from(env::var("KORDRA_DATA_DIR").unwrap_or_else(|_| "./data".to_owned()));
    let downloads = PathBuf::from(env::var("KORDRA_DOWNLOAD_DIR").unwrap_or_else(|_| "./downloads".to_owned()));
    let db = Database::open(&data).await?;
    let manager = YtDlpManager::new(data.clone())?;
    let deno = DenoManager::new(data)?;
    let worker = Worker::new(db.clone(), manager.clone(), deno.clone(), downloads, 2);
    worker.enqueue_pending().await?;
    let updater = manager.clone();
    tokio::spawn(async move {
        loop {
            match updater.state().await {
                Ok(policy) if policy.auto_update && policy.pinned.is_none() && policy.active.is_some() => {
                    if let Err(error) = updater.update(policy.channel, true).await {
                        tracing::warn!("yt-dlp auto-update failed: {error:#}");
                    }
                }
                Ok(_) => {}
                Err(error) => tracing::warn!("could not read yt-dlp update policy: {error:#}"),
            }
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        }
    });
    let deno_updater = deno.clone();
    tokio::spawn(async move {
        loop {
            match deno_updater.state().await {
                Ok(policy) if policy.auto_update && policy.pinned.is_none() && policy.active.is_some() => {
                    if let Err(error) = deno_updater.update(true).await {
                        tracing::warn!("Deno auto-update failed: {error:#}");
                    }
                }
                Ok(_) => {}
                Err(error) => tracing::warn!("could not read Deno update policy: {error:#}"),
            }
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        }
    });
    let state = ApiState { db, manager, deno, worker, token, metrics_token };
    tracing::info!("Kordra listening on {address}");
    let listener = TcpListener::bind(address).await?;
    axum::serve(listener, router(state)).await?;
    Ok(())
}
