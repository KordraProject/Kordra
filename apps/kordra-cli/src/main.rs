use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "kordra-cli", version, about = "Kordra headless API client")]
struct Options {
    #[arg(long, env = "KORDRA_URL", default_value = "http://127.0.0.1:8079")]
    url: String,
    #[arg(long, env = "KORDRA_API_TOKEN")]
    token: Option<String>,
    #[command(subcommand)]
    command: Operation,
}
#[derive(Subcommand)]
enum Operation {
    Health,
    Stats,
    List,
    Add { source: String, #[arg(long, default_value = "yt-dlp")] engine: String },
    EngineState,
    UpdateYtdlp { #[arg(long, default_value = "stable")] channel: String },
    InstallYtdlp { version: String, #[arg(long, default_value = "stable")] channel: String },
    ActivateYtdlp { version: String },
    RollbackYtdlp,
    DenoState,
    UpdateDeno,
    InstallDeno { version: String },
    ActivateDeno { version: String },
    RollbackDeno,
    EngineLifecycle,
    SetLifecycle { #[arg(long)] startup: String, #[arg(long, default_value = "persistent")] cold_retention: String },
    PrepareEngine { id: String },
    Pause { id: String },
    Resume { id: String },
    Retry { id: String },
    Cancel { id: String },
}
#[tokio::main]
async fn main() -> Result<()> {
    let opts = Options::parse();
    let url = format!("{}/api/v1", opts.url.trim_end_matches('/'));
    let (post, route, body) = match opts.command {
        Operation::Health => (false, "/health".into(), None),
        Operation::Stats => (false, "/stats".into(), None),
        Operation::List => (false, "/downloads".into(), None),
        Operation::Add { source, engine } =>
            (true, "/downloads".into(), Some(serde_json::json!({"source":source,"engine":engine}))),
        Operation::EngineState => (false, "/engines/yt-dlp".into(), None),
        Operation::UpdateYtdlp { channel } => (true, "/engines/yt-dlp/update".into(),
            Some(serde_json::json!({"channel":channel,"activate":true}))),
        Operation::InstallYtdlp { version, channel } => (true, format!("/engines/yt-dlp/install/{version}"),
            Some(serde_json::json!({"channel":channel,"activate":false}))),
        Operation::ActivateYtdlp { version } => (true, format!("/engines/yt-dlp/activate/{version}"), None),
        Operation::RollbackYtdlp => (true, "/engines/yt-dlp/rollback".into(), None),
        Operation::DenoState => (false, "/runtimes/deno".into(), None),
        Operation::UpdateDeno => (true, "/runtimes/deno/update".into(), Some(serde_json::json!({"activate":true}))),
        Operation::InstallDeno { version } => (true, format!("/runtimes/deno/install/{version}"), Some(serde_json::json!({"activate":false}))),
        Operation::ActivateDeno { version } => (true, format!("/runtimes/deno/activate/{version}"), None),
        Operation::RollbackDeno => (true, "/runtimes/deno/rollback".into(), None),
        Operation::EngineLifecycle => (false, "/engines/lifecycle".into(), None),
        Operation::SetLifecycle { startup, cold_retention } => (true, "/engines/lifecycle".into(),
            Some(serde_json::json!({"startup":startup,"cold_retention":cold_retention}))),
        Operation::PrepareEngine { id } => (true, format!("/engines/{id}/prepare"), None),
        Operation::Pause { id } => (true, format!("/downloads/{id}/pause"), None),
        Operation::Resume { id } => (true, format!("/downloads/{id}/resume"), None),
        Operation::Retry { id } => (true, format!("/downloads/{id}/retry"), None),
        Operation::Cancel { id } => (true, format!("/downloads/{id}/cancel"), None),
    };
    let client = reqwest::Client::new();
    let request = if route == "/engines/lifecycle" && post { client.put(format!("{url}{route}")) }
        else if post { client.post(format!("{url}{route}")) } else { client.get(format!("{url}{route}")) };
    let request = if let Some(token) = opts.token { request.bearer_auth(token) } else { request };
    let request = if let Some(body) = body { request.json(&body) } else { request };
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() { anyhow::bail!("HTTP {status}: {text}"); }
    println!("{text}");
    Ok(())
}
