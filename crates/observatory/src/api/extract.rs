//! Extracteurs : session, origine, client, corps et chemins validés.

use std::net::{IpAddr, SocketAddr};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::request::Parts;
use axum::http::{Method, StatusCode, header};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use super::AppState;
use crate::error::ApiError;
use crate::security::{check_origin, csrf_matches, session_cookie, token_hash};

/// Analyste authentifié. Les méthodes modifiantes exigent aussi l'origine et le jeton CSRF.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: Uuid,
    pub login: String,
    pub token: String,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = session_cookie(&parts.headers).unwrap_or_default();
        let user: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT u.id, u.login FROM users u JOIN sessions s ON s.user_id = u.id \
             WHERE s.token_hash = $1 AND s.expires_at > now()",
        )
        .bind(token_hash(&token))
        .fetch_optional(&state.pool)
        .await?;
        let Some((id, login)) = user else {
            return Err(ApiError::unauthorized("Connexion requise"));
        };
        if !matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS) {
            check_origin(&parts.headers, &state.config.origin.origin)?;
            let provided =
                parts.headers.get("x-csrf-token").and_then(|value| value.to_str().ok()).unwrap_or("");
            if !csrf_matches(&token, provided) {
                return Err(ApiError::forbidden("Jeton CSRF incorrect"));
            }
        }
        Ok(Self { id, login, token })
    }
}

/// Requête venue de l'origine publique (ou sans en-tête `Origin`).
#[derive(Debug, Clone, Copy)]
pub struct SameOrigin;

impl FromRequestParts<AppState> for SameOrigin {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, ApiError>> + Send {
        std::future::ready(check_origin(&parts.headers, &state.config.origin.origin).map(|()| Self))
    }
}

/// Adresse du client : `X-Real-IP` derrière le proxy HTTPS, sinon l'adresse TCP.
#[derive(Debug, Clone)]
pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, ApiError>> + Send {
        let forwarded = parts
            .headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
            .filter(|_| state.config.trust_proxy_headers);
        let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|ConnectInfo(address)| address.ip());
        let client = forwarded.or(peer).map(|ip| ip.to_string()).unwrap_or_default();
        std::future::ready(Ok(Self(client)))
    }
}

/// Règles de longueur et de format d'un corps JSON.
pub trait Validate {
    fn validate(&self) -> Result<(), String>;
}

/// Corps JSON désérialisé puis validé : 415 si ce n'est pas du JSON, 422 s'il est invalide.
#[derive(Debug)]
pub struct ValidJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Validate> FromRequest<S> for ValidJson<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        let json = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"));
        if !json {
            return Err(ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON requis"));
        }
        let body = Bytes::from_request(request, state)
            .await
            .map_err(|_| ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "Requête trop volumineuse"))?;
        let value: T = serde_json::from_slice(&body)
            .map_err(|error| ApiError::unprocessable(format!("Requête invalide : {error}")))?;
        value.validate().map_err(ApiError::unprocessable)?;
        Ok(Self(value))
    }
}

/// Paramètres de chemin ; un identifiant mal formé donne 422 comme au socle 0.1.
#[derive(Debug)]
pub struct ApiPath<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for ApiPath<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|_| ApiError::unprocessable("Identifiant invalide"))
    }
}

#[derive(Debug)]
pub struct ApiQuery<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|error| ApiError::unprocessable(format!("Paramètre invalide : {error}")))
    }
}
