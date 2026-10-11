//! Dossiers d'investigation, membres et notes.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use observatory_domain::text::within;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::AppState;
use super::extract::{ApiPath, CurrentUser, ValidJson, Validate};
use crate::error::{ApiError, ApiResult};
use crate::model::Investigation;
use crate::security::{Access, audit, require_access};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct InvestigationSummary {
    id: Uuid,
    title: String,
    description: String,
    created_at: DateTime<Utc>,
    role: String,
    scan_count: i64,
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Vec<InvestigationSummary>>> {
    let rows = sqlx::query_as(
        "SELECT i.id, i.title, i.description, i.created_at, m.role, \
         (SELECT count(*) FROM scans s WHERE s.investigation_id = i.id) AS scan_count \
         FROM investigations i JOIN memberships m ON m.investigation_id = i.id \
         WHERE m.user_id = $1 ORDER BY i.created_at DESC LIMIT 200",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct InvestigationInput {
    title: String,
    #[serde(default)]
    description: String,
}

impl Validate for InvestigationInput {
    fn validate(&self) -> Result<(), String> {
        if !within(&self.title, 1, 160) {
            return Err("Titre : 1 à 160 caractères".into());
        }
        if !within(&self.description, 0, 5000) {
            return Err("Objectif : 5000 caractères au plus".into());
        }
        Ok(())
    }
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    ValidJson(data): ValidJson<InvestigationInput>,
) -> ApiResult<(StatusCode, Json<Investigation>)> {
    let mut tx = state.pool.begin().await?;
    let row: Investigation = sqlx::query_as(
        "INSERT INTO investigations(id, title, description) VALUES($1, $2, $3) \
         RETURNING id, title, description, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(&data.title)
    .bind(&data.description)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO memberships VALUES($1, $2, 'owner')")
        .bind(row.id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, user.id, "investigation.create", Some(row.id)).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Member {
    login: String,
    role: String,
}

pub async fn members(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<Member>>> {
    let mut conn = state.pool.acquire().await?;
    require_access(&mut conn, investigation_id, user.id, Access::Read).await?;
    let rows = sqlx::query_as(
        "SELECT u.login, m.role FROM memberships m JOIN users u ON u.id = m.user_id \
         WHERE m.investigation_id = $1 ORDER BY u.login",
    )
    .bind(investigation_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemberRole {
    Analyst,
    Reader,
}

#[derive(Debug, Deserialize)]
pub struct MemberInput {
    login: String,
    role: MemberRole,
}

impl Validate for MemberInput {
    fn validate(&self) -> Result<(), String> {
        if within(&self.login, 3, 40) { Ok(()) } else { Err("Identifiant : 3 à 40 caractères".into()) }
    }
}

pub async fn add_member(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
    ValidJson(data): ValidJson<MemberInput>,
) -> ApiResult<Json<Value>> {
    let mut tx = state.pool.begin().await?;
    require_access(&mut tx, investigation_id, user.id, Access::Owner).await?;
    let target: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE login = $1")
        .bind(data.login.to_lowercase())
        .fetch_optional(&mut *tx)
        .await?;
    let Some(target) = target else {
        return Err(ApiError::not_found("Compte local introuvable"));
    };
    if target == user.id {
        return Err(ApiError::conflict("Le propriétaire conserve son rôle"));
    }
    let role = match data.role {
        MemberRole::Analyst => "analyst",
        MemberRole::Reader => "reader",
    };
    sqlx::query(
        "INSERT INTO memberships VALUES($1, $2, $3) ON CONFLICT(investigation_id, user_id) \
         DO UPDATE SET role = excluded.role",
    )
    .bind(investigation_id)
    .bind(target)
    .bind(role)
    .execute(&mut *tx)
    .await?;
    audit(&mut tx, user.id, "membership.update", Some(investigation_id)).await?;
    tx.commit().await?;
    Ok(Json(json!({"status": "ok"})))
}

pub async fn remove_member(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath((investigation_id, login)): ApiPath<(Uuid, String)>,
) -> ApiResult<StatusCode> {
    let mut tx = state.pool.begin().await?;
    require_access(&mut tx, investigation_id, user.id, Access::Owner).await?;
    let removed = sqlx::query(
        "DELETE FROM memberships WHERE investigation_id = $1 AND role <> 'owner' \
         AND user_id IN (SELECT id FROM users WHERE login = $2)",
    )
    .bind(investigation_id)
    .bind(login.to_lowercase())
    .execute(&mut *tx)
    .await?;
    if removed.rows_affected() > 0 {
        audit(&mut tx, user.id, "membership.remove", Some(investigation_id)).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Note {
    id: Uuid,
    body: String,
    created_at: DateTime<Utc>,
    login: String,
}

pub async fn notes(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<Note>>> {
    let mut conn = state.pool.acquire().await?;
    require_access(&mut conn, investigation_id, user.id, Access::Read).await?;
    let rows = sqlx::query_as(
        "SELECT a.id, a.body, a.created_at, u.login FROM annotations a JOIN users u ON u.id = a.author_id \
         WHERE a.investigation_id = $1 ORDER BY a.created_at DESC LIMIT 100",
    )
    .bind(investigation_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct NoteInput {
    body: String,
}

impl Validate for NoteInput {
    fn validate(&self) -> Result<(), String> {
        if within(&self.body, 1, 10_000) { Ok(()) } else { Err("Note : 1 à 10 000 caractères".into()) }
    }
}

pub async fn add_note(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
    ValidJson(data): ValidJson<NoteInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let mut tx = state.pool.begin().await?;
    require_access(&mut tx, investigation_id, user.id, Access::Write).await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO annotations(id, investigation_id, author_id, body) VALUES($1, $2, $3, $4) RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(investigation_id)
    .bind(user.id)
    .bind(&data.body)
    .fetch_one(&mut *tx)
    .await?;
    audit(&mut tx, user.id, "note.create", Some(investigation_id)).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}
