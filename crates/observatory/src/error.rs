//! Erreurs HTTP : `{"detail": "..."}`, comme le socle 0.1 que lit le frontend.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Refus destiné à l'analyste : le message est affiché tel quel.
    #[error("{1}")]
    Client(StatusCode, String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, detail: impl Into<String>) -> Self {
        Self::Client(status, detail.into())
    }

    pub fn unauthorized(detail: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, detail)
    }

    pub fn forbidden(detail: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, detail)
    }

    pub fn not_found(detail: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, detail)
    }

    pub fn conflict(detail: &str) -> Self {
        Self::new(StatusCode::CONFLICT, detail)
    }

    pub fn unprocessable(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, detail)
    }

    pub fn too_many(detail: &str) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, detail)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, detail) = match self {
            Self::Client(status, detail) => (status, detail),
            // Provider payloads and SQL never reach the client; the log has the cause.
            Self::Database(sqlx::Error::PoolTimedOut) => {
                tracing::warn!("database pool exhausted");
                (StatusCode::SERVICE_UNAVAILABLE, "Service momentanément indisponible".to_owned())
            }
            Self::Database(error) => {
                tracing::error!(%error, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Erreur interne ; consulter le journal de l'API".to_owned(),
                )
            }
            Self::Internal(error) => {
                tracing::error!(error = format!("{error:#}"), "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Erreur interne ; consulter le journal de l'API".to_owned(),
                )
            }
        };
        (status, Json(json!({ "detail": detail }))).into_response()
    }
}
