//! Versioned REST API. Local-only by default. Public TLS and OIDC are NOT ready.
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use kordra_core::{Database, Download, NewDownload, Statistics};
use kordra_engine_manager::{Channel, VersionState, YtDlpManager};
use kordra_worker::Worker;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct ApiState {
    pub db: Database,
    pub manager: YtDlpManager,
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
        .route("/stats", get(stats))
        .route("/engines/yt-dlp", get(engine_state))
        .route("/engines/yt-dlp/update", post(update_engine))
        .route("/engines/yt-dlp/install/{version}", post(install_engine_version))
        .route("/engines/yt-dlp/activate/{version}", post(activate_engine))
        .route("/engines/yt-dlp/rollback", post(rollback_engine))
        .route("/engines/yt-dlp/policy", post(engine_policy))
        .route_layer(middleware::from_fn_with_state(state.clone(), api_auth));
    let app = Router::new().nest("/api/v1", api)
        .route("/metrics", get(metrics_endpoint))
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
async fn new_download(State(state): State<ApiState>, Json(req): Json<NewDownload>) -> ApiResult<(StatusCode, Json<Download>)> {
    if req.engine != "yt-dlp" {
        return Err((StatusCode::NOT_IMPLEMENTED, Json(ErrorBody { error: "engine adapter not implemented yet".into() })));
    }
    let task = state.db.create(req).await.map_err(invalid)?;
    state.worker.enqueue(task.clone());
    Ok((StatusCode::CREATED, Json(task)))
}
async fn stats(State(state): State<ApiState>) -> ApiResult<Json<Statistics>> {
    state.db.stats().await.map(Json).map_err(internal)
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
async fn metrics_endpoint(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    if state.metrics_token.as_ref().or(state.token.as_ref())
        .is_some_and(|token| !bearer_matches(&headers, token)) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let stats = match state.db.stats().await {
        Ok(value) => value,
        Err(error) => return internal(error).into_response(),
    };
    let body = format!(
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
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")], body).into_response()
}
