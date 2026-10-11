//! Connexion, session courante et déconnexion.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use observatory_domain::text::within;
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use super::extract::{ClientIp, CurrentUser, SameOrigin, ValidJson, Validate};
use crate::error::ApiResult;
use crate::security::{COOKIE, SESSION_SECONDS, authenticate, csrf_token, token_hash};

#[derive(Debug, Deserialize)]
pub struct Login {
    login: String,
    password: String,
}

impl Validate for Login {
    fn validate(&self) -> Result<(), String> {
        if !within(&self.login, 3, 40) {
            return Err("Identifiant : 3 à 40 caractères".into());
        }
        if !within(&self.password, 1, 1024) {
            return Err("Mot de passe : 1 à 1024 caractères".into());
        }
        Ok(())
    }
}

fn session_cookie(state: &AppState, value: &str, max_age: i64) -> HeaderValue {
    let secure = if state.config.origin.secure { "; Secure" } else { "" };
    let cookie = format!("{COOKIE}={value}; HttpOnly; Max-Age={max_age}; Path=/; SameSite=strict{secure}");
    HeaderValue::from_str(&cookie).expect("session tokens are URL-safe")
}

pub async fn login(
    State(state): State<AppState>,
    _: SameOrigin,
    ClientIp(client): ClientIp,
    ValidJson(data): ValidJson<Login>,
) -> ApiResult<Response> {
    let (user, token) = authenticate(&state.pool, &data.login, &data.password, &client).await?;
    let body = Json(json!({"id": user.id, "login": user.login, "csrf": csrf_token(&token)}));
    let mut response = body.into_response();
    response.headers_mut().insert(header::SET_COOKIE, session_cookie(&state, &token, SESSION_SECONDS));
    Ok(response)
}

pub async fn me(user: CurrentUser) -> Json<serde_json::Value> {
    Json(json!({"id": user.id, "login": user.login, "csrf": csrf_token(&user.token)}))
}

pub async fn logout(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Response> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash(&user.token))
        .execute(&state.pool)
        .await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(header::SET_COOKIE, session_cookie(&state, "\"\"", 0));
    Ok(response)
}
