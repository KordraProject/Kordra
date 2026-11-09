//! Versioned REST API. Local-only by default. Public TLS and OIDC are NOT ready.
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use kordra_core::{DailyTransfer, Database, Download, EngineTransfer, NewDownload, Statistics};
use kordra_engine_manager::{Channel, DenoManager, DenoState, VersionState, YtDlpManager};
use kordra_worker::{EngineLifecyclePolicy, Worker};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct ApiState {
    pub db: Database,
    pub manager: YtDlpManager,
    pub deno: DenoManager,
    pub worker: Worker,
    pub token: Option<String>,
    pub metrics_token: Option<String>,
}
type ApiResult<T> = Result<T, ApiError>;
type ApiError = (StatusCode, Json<ErrorBody>);
#[derive(Serialize)]
struct ErrorBody { error: String }
fn invalid(message: impl std::fmt::Display) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(ErrorBody { error: message.to_string() }))
}
fn internal(error: impl std::fmt::Display) -> ApiError {
    tracing::error!("API internal error: {error}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody { error: "internal error".into() }))
}
pub fn router(state: ApiState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/downloads", get(list_downloads).post(new_download))
        .route("/downloads/{id}", get(get_download))
        .route("/downloads/{id}/pause", post(pause_download))
        .route("/downloads/{id}/torrent", get(torrent_details))
        .route("/downloads/{id}/torrent/files", post(select_torrent_files))
        .route("/downloads/{id}/resume", post(resume_download))
        .route("/downloads/{id}/retry", post(retry_download))
        .route("/downloads/{id}/cancel", post(cancel_download))
        .route("/downloads/{id}/stop-seeding", post(stop_seeding))
        .route("/engines/capabilities", get(capabilities))
        .route("/engines/lifecycle", get(engine_lifecycle).put(set_engine_lifecycle))
        .route("/engines/{id}/prepare", post(prepare_engine))
        .route("/queue", get(queue_status))
        .route("/stats", get(stats))
        .route("/stats/daily", get(daily_stats))
        .route("/stats/engines", get(engine_stats))
        .route("/engines/yt-dlp", get(engine_state))
        .route("/engines/yt-dlp/update", post(update_engine))
        .route("/engines/yt-dlp/install/{version}", post(install_engine_version))
        .route("/engines/yt-dlp/activate/{version}", post(activate_engine))
        .route("/engines/yt-dlp/rollback", post(rollback_engine))
        .route("/engines/yt-dlp/policy", post(engine_policy))
        .route("/runtimes/deno", get(deno_state))
        .route("/runtimes/deno/update", post(update_deno))
        .route("/runtimes/deno/install/{version}", post(install_deno))
        .route("/runtimes/deno/activate/{version}", post(activate_deno))
        .route("/runtimes/deno/rollback", post(rollback_deno))
        .route("/runtimes/deno/policy", post(deno_policy))
        .route_layer(middleware::from_fn_with_state(state.clone(), api_auth));
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin([
            axum::http::HeaderValue::from_static("tauri://localhost"),
            axum::http::HeaderValue::from_static("http://tauri.localhost"),
        ])
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([axum::http::header::AUTHORIZATION, axum::http::header::CONTENT_TYPE]);
    let app = Router::new().nest("/api/v1", api)
        .route("/metrics", get(metrics_endpoint))
        .layer(cors)
        .with_state(state);
    let enabled = std::env::var("KORDRA_WEB_ENABLED")
        .map(|v| v != "false" && v != "0").unwrap_or(true);
    if enabled {
        let web_dir = std::env::var("KORDRA_WEB_DIR").unwrap_or_else(|_| "./apps/web/dist".into());
        app.fallback_service(tower_http::services::ServeDir::new(web_dir))
    } else {
        app
    }
}
fn bearer_matches(headers: &HeaderMap, expected: &str) -> bool {
    let Some(candidate) = headers.get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer ")) else { return false; };
    candidate.as_bytes().ct_eq(expected.as_bytes()).into()
}
async fn api_auth(State(state): State<ApiState>, headers: HeaderMap, request: axum::extract::Request, next: Next) -> Response {
    if state.token.as_ref().is_some_and(|expected| !bearer_matches(&headers, expected)) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","version":env!("CARGO_PKG_VERSION")}))
}
async fn list_downloads(State(state): State<ApiState>) -> ApiResult<Json<Vec<Download>>> {
    state.db.list().await.map(Json).map_err(internal)
}
async fn get_download(Path(id): Path<String>, State(state): State<ApiState>) -> ApiResult<Json<Download>> {
    state.db.get(&id).await.map_err(internal)?
        .map(Json).ok_or_else(|| (StatusCode::NOT_FOUND, Json(ErrorBody { error: "task not found".into() })))
}
#[derive(Deserialize)]
struct TorrentFileSelection { only_files: Vec<usize> }
async fn select_torrent_files(Path(id): Path<String>, State(state): State<ApiState>, Json(req): Json<TorrentFileSelection>) -> ApiResult<Json<serde_json::Value>> {
    state.worker.select_torrent_files(&id, &req.only_files).await.map(Json).map_err(invalid)
}
async fn torrent_details(Path(id): Path<String>, State(state): State<ApiState>) -> ApiResult<Json<serde_json::Value>> {
    state.worker.torrent_details(&id).await.map(Json).map_err(|_| {
        (StatusCode::NOT_FOUND, Json(ErrorBody { error: "no active torrent session for this task".into() }))
    })
}
async fn pause_download(Path(id): Path<String>, State(state): State<ApiState>) -> ApiResult<Json<Download>> {
    state.worker.pause(&id).await.map(Json).map_err(invalid)
}
async fn resume_download(Path(id): Path<String>, State(state): State<ApiState>) -> ApiResult<Json<Download>> {
    state.worker.resume(&id).await.map(Json).map_err(invalid)
}
async fn stop_seeding(Path(id): Path<String>, State(state): State<ApiState>) -> ApiResult<Json<Download>> {
    state.worker.stop_seeding(&id).await.map(Json).map_err(|error| {
        (StatusCode::CONFLICT, Json(ErrorBody { error: error.to_string() }))
    })
}
async fn cancel_download(Path(id): Path<String>, State(state): State<ApiState>) -> ApiResult<Json<Download>> {
    state.worker.cancel(&id).await.map(Json).map_err(|error| {
        (StatusCode::CONFLICT, Json(ErrorBody { error: error.to_string() }))
    })
}
async fn retry_download(Path(id): Path<String>, State(state): State<ApiState>) -> ApiResult<Json<Download>> {
    state.worker.retry(&id).await.map(Json).map_err(|error| {
        (StatusCode::CONFLICT, Json(ErrorBody { error: error.to_string() }))
    })
}
async fn new_download(State(state): State<ApiState>, Json(req): Json<NewDownload>) -> ApiResult<(StatusCode, Json<Download>)> {
    if req.engine != "yt-dlp" && req.engine != "librqbit" {
        return Err((StatusCode::NOT_IMPLEMENTED, Json(ErrorBody { error: "engine adapter not implemented yet".into() })));
    }
    let task = state.db.create(req).await.map_err(invalid)?;
    state.worker.enqueue(task.clone());
    Ok((StatusCode::CREATED, Json(task)))
}
async fn engine_lifecycle(State(state): State<ApiState>) -> ApiResult<Json<serde_json::Value>> {
    state.worker.lifecycle_status().await.map(Json).map_err(internal)
}
async fn set_engine_lifecycle(State(state): State<ApiState>, Json(policy): Json<EngineLifecyclePolicy>) -> ApiResult<Json<serde_json::Value>> {
    state.worker.set_lifecycle_policy(policy).await.map_err(invalid)?;
    state.worker.lifecycle_status().await.map(Json).map_err(internal)
}
async fn prepare_engine(Path(id): Path<String>, State(state): State<ApiState>) -> ApiResult<Json<serde_json::Value>> {
    state.worker.prepare_engine(&id).await.map(Json).map_err(invalid)
}
async fn capabilities() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "engines": [
            {"id":"yt-dlp","name":"yt-dlp","installed":"on_demand","sources":["http","https"],"capabilities":["download","versions"]},
            {"id":"librqbit","name":"librqbit","installed":"embedded","sources":["magnet","http","https"],"capabilities":["download","seed","transfer_stats"]}
        ]
    }))
}
async fn queue_status(State(state): State<ApiState>) -> ApiResult<Json<serde_json::Value>> {
    state.worker.queue_status().await.map(Json).map_err(internal)
}
async fn stats(State(state): State<ApiState>) -> ApiResult<Json<Statistics>> {
    state.db.stats().await.map(Json).map_err(internal)
}
async fn daily_stats(State(state): State<ApiState>) -> ApiResult<Json<Vec<DailyTransfer>>> {
    state.db.daily_stats().await.map(Json).map_err(internal)
}
async fn engine_stats(State(state): State<ApiState>) -> ApiResult<Json<Vec<EngineTransfer>>> {
    state.db.engine_stats().await.map(Json).map_err(internal)
}
async fn engine_state(State(state): State<ApiState>) -> ApiResult<Json<VersionState>> {
    state.manager.state().await.map(Json).map_err(internal)
}
#[derive(Deserialize)]
struct UpdateRequest { channel: Option<Channel>, activate: Option<bool> }
async fn update_engine(State(state): State<ApiState>, Json(req): Json<UpdateRequest>) -> ApiResult<Json<VersionState>> {
    let current = state.manager.state().await.map_err(internal)?;
    state.manager.update(req.channel.unwrap_or(current.channel), req.activate.unwrap_or(true)).await
        .map(Json).map_err(invalid)
}
async fn install_engine_version(Path(version): Path<String>, State(state): State<ApiState>, Json(req): Json<UpdateRequest>) -> ApiResult<Json<VersionState>> {
    let current = state.manager.state().await.map_err(internal)?;
    state.manager.install_version(req.channel.unwrap_or(current.channel), &version, req.activate.unwrap_or(false)).await
        .map(Json).map_err(invalid)
}
async fn activate_engine(Path(version): Path<String>, State(state): State<ApiState>) -> ApiResult<Json<VersionState>> {
    state.manager.activate(&version).await.map(Json).map_err(invalid)
}
async fn rollback_engine(State(state): State<ApiState>) -> ApiResult<Json<VersionState>> {
    state.manager.rollback().await.map(Json).map_err(invalid)
}
#[derive(Deserialize)]
struct Policy { channel: Channel, pinned: Option<String>, auto_update: bool }
async fn engine_policy(State(state): State<ApiState>, Json(req): Json<Policy>) -> ApiResult<Json<VersionState>> {
    state.manager.set_policy(req.channel, req.pinned, req.auto_update).await.map(Json).map_err(invalid)
}
async fn deno_state(State(state): State<ApiState>) -> ApiResult<Json<DenoState>> {
    state.deno.state().await.map(Json).map_err(internal)
}
#[derive(Deserialize)]
struct RuntimeInstall { activate: Option<bool> }
async fn update_deno(State(state): State<ApiState>, Json(req): Json<RuntimeInstall>) -> ApiResult<Json<DenoState>> {
    state.deno.update(req.activate.unwrap_or(true)).await.map(Json).map_err(invalid)
}
async fn install_deno(Path(version): Path<String>, State(state): State<ApiState>, Json(req): Json<RuntimeInstall>) -> ApiResult<Json<DenoState>> {
    state.deno.install_version(&version, req.activate.unwrap_or(false)).await.map(Json).map_err(invalid)
}
async fn activate_deno(Path(version): Path<String>, State(state): State<ApiState>) -> ApiResult<Json<DenoState>> {
    state.deno.activate(&version).await.map(Json).map_err(invalid)
}
async fn rollback_deno(State(state): State<ApiState>) -> ApiResult<Json<DenoState>> {
    state.deno.rollback().await.map(Json).map_err(invalid)
}
#[derive(Deserialize)]
struct DenoPolicy { pinned: Option<String>, auto_update: bool }
async fn deno_policy(State(state): State<ApiState>, Json(req): Json<DenoPolicy>) -> ApiResult<Json<DenoState>> {
    state.deno.policy(req.pinned, req.auto_update).await.map(Json).map_err(invalid)
}
async fn metrics_endpoint(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    if state.metrics_token.as_ref().or(state.token.as_ref())
        .is_some_and(|token| !bearer_matches(&headers, token)) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let stats = match state.db.stats().await {
        Ok(value) => value,
        Err(error) => return internal(error).into_response(),
    };
    let by_engine = match state.db.engine_stats().await {
        Ok(items) => items,
        Err(error) => return internal(error).into_response(),
    };
    let mut body = format!(
        "# HELP kordra_download_bytes_total Total bytes downloaded, persisted in SQLite\n\
         # TYPE kordra_download_bytes_total counter\n\
         kordra_download_bytes_total {}\n\
         # HELP kordra_upload_bytes_total Total bytes uploaded, persisted in SQLite\n\
         # TYPE kordra_upload_bytes_total counter\n\
         kordra_upload_bytes_total {}\n\
         # HELP kordra_tasks_total Tasks ever created\n\
         # TYPE kordra_tasks_total gauge\n\
         kordra_tasks_total {}\n\
         # HELP kordra_active_tasks Running tasks\n\
         # TYPE kordra_active_tasks gauge\n\
         kordra_active_tasks {}\n\
         # HELP kordra_completed_tasks_total Completed tasks\n\
         # TYPE kordra_completed_tasks_total counter\n\
         kordra_completed_tasks_total {}\n\
         # HELP kordra_failed_tasks_total Failed tasks\n\
         # TYPE kordra_failed_tasks_total counter\n\
         kordra_failed_tasks_total {}\n",
        stats.total_downloaded_bytes, stats.total_uploaded_bytes, stats.total_tasks,
        stats.active_tasks, stats.completed_tasks, stats.failed_tasks
    );
    body.push_str("# HELP kordra_engine_download_bytes_total Downloaded bytes by engine\n# TYPE kordra_engine_download_bytes_total counter\n");
    body.push_str("# HELP kordra_engine_upload_bytes_total Uploaded bytes by engine\n# TYPE kordra_engine_upload_bytes_total counter\n");
    for engine in by_engine {
        // Engine IDs come from a strict allowlist in task validation; defensive escape anyway.
        let label = engine.engine.replace('\\', "\\\\").replace('"', "\\\"");
        body.push_str(&format!("kordra_engine_download_bytes_total{{engine=\"{label}\"}} {}\n", engine.downloaded_bytes));
        body.push_str(&format!("kordra_engine_upload_bytes_total{{engine=\"{label}\"}} {}\n", engine.uploaded_bytes));
    }
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")], body).into_response()
}
