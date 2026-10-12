//! API HTTP et service de l'interface compilée.

mod auth;
mod discovery;
mod extract;
mod graph;
mod investigations;
mod keywords;
pub(crate) mod scans;
mod triage;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde_json::json;
use sqlx::PgPool;
use tower_http::services::ServeDir;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::config::ApiConfig;
use crate::error::ApiError;

#[derive(Debug, Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<ApiConfig>,
}

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/login", post(auth::login))
        .route("/me", get(auth::me))
        .route("/logout", post(auth::logout))
        .route("/investigations", get(investigations::list).post(investigations::create))
        .route(
            "/investigations/{investigation_id}/members",
            get(investigations::members).put(investigations::add_member),
        )
        .route("/investigations/{investigation_id}/members/{login}", delete(investigations::remove_member))
        .route(
            "/investigations/{investigation_id}/notes",
            get(investigations::notes).post(investigations::add_note),
        )
        .route("/investigations/{investigation_id}/scans", get(scans::list).post(scans::create))
        .route("/investigations/{investigation_id}/scans/batch", post(scans::create_batch))
        .route("/profiles", get(scans::profiles))
        .route("/investigations/{investigation_id}/discoveries", post(discovery::create))
        .route("/investigations/{investigation_id}/discoveries/preview", post(discovery::preview))
        .route("/investigations/{investigation_id}/keyword-lists", get(keywords::list).post(keywords::create))
        .route("/keyword-lists/preview", post(keywords::preview))
        .route("/keyword-lists/{list_id}/versions", post(keywords::publish_version))
        .route("/keyword-lists/{list_id}/versions/{version}", get(keywords::version))
        .route("/keyword-lists/{list_id}/versions/{version}/runs", post(triage::create))
        .route("/investigations/{investigation_id}/rule-runs", get(triage::list))
        .route("/rule-runs/{run_id}/cancel", post(triage::cancel))
        .route("/investigations/{investigation_id}/triage", get(triage::queue))
        .route("/triage/{item_id}", put(triage::decide))
        .route("/investigations/{investigation_id}/graph", get(graph::investigation_graph))
        .route("/investigations/{investigation_id}/common", get(graph::common))
        .route("/search", get(graph::search))
        .route("/scans/{scan_id}/cancel", post(scans::cancel))
        .route("/scans/{scan_id}/graph", get(graph::graph))
        .route("/scans/{scan_id}/compare/{other_id}", get(graph::compare))
        .route("/entities/{entity_id}/overlaps", get(graph::overlaps))
        .route("/evidence/{digest}", get(graph::evidence));
    let assets = ServeDir::new(state.config.static_dir.join("assets"));
    Router::new()
        .route("/health", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/ready", get(ready))
        .route("/", get(index))
        .nest("/api/v1", api)
        .nest_service("/assets", assets)
        .fallback(|| async { ApiError::not_found("Not Found") })
        .layer(middleware::from_fn_with_state(state.clone(), browser_policy))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn ready(State(state): State<AppState>) -> Response {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => Json(json!({"status": "ready"})).into_response(),
        Err(error) => {
            tracing::warn!(%error, "readiness check failed");
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "Base indisponible").into_response()
        }
    }
}

async fn index(State(state): State<AppState>) -> Result<Html<Vec<u8>>, ApiError> {
    tokio::fs::read(state.config.static_dir.join("index.html"))
        .await
        .map(Html)
        .map_err(|_| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "Interface non construite"))
}

/// Hôte attendu, puis en-têtes de sécurité sur toutes les réponses.
async fn browser_policy(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| request.uri().authority().map(axum::http::uri::Authority::as_str))
        .unwrap_or_default();
    let hostname = match host.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    }
    .to_ascii_lowercase();
    let mut response = if state.config.allowed_hosts().contains(&hostname.as_str()) {
        next.run(request).await
    } else {
        ApiError::new(StatusCode::BAD_REQUEST, "Invalid host header").into_response()
    };
    let headers = response.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CONTENT_SECURITY_POLICY));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if state.config.origin.secure {
        headers.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000"));
    }
    response
}

/// Sert l'API jusqu'au signal d'arrêt, en terminant les requêtes en cours.
pub async fn serve(
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(state.config.listen).await?;
    tracing::info!(address = %state.config.listen, origin = %state.config.origin.origin, "API listening");
    axum::serve(listener, router(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}
