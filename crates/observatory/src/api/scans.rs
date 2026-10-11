//! Demande, liste et annulation des scans.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use observatory_domain::parse_seed;
use observatory_domain::text::within;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::AppState;
use super::extract::{ApiPath, CurrentUser, ValidJson, Validate};
use crate::error::{ApiError, ApiResult};
use crate::model::{PENDING_STATUSES, Scan};
use crate::security::{Access, accessible_scan, audit, require_access};

/// Scans en attente ou actifs tolérés par investigation.
pub const MAX_PENDING_SCANS: i64 = 3;

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<Scan>>> {
    let mut conn = state.pool.acquire().await?;
    require_access(&mut conn, investigation_id, user.id, Access::Read).await?;
    let rows = sqlx::query_as(
        "SELECT * FROM scans WHERE investigation_id = $1 ORDER BY requested_at DESC LIMIT 100",
    )
    .bind(investigation_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Json(rows))
}

fn default_budget() -> i32 {
    12
}

#[derive(Debug, Deserialize)]
pub struct ScanInput {
    seed: String,
    #[serde(default = "default_budget")]
    max_requests: i32,
    #[serde(default = "Uuid::new_v4")]
    idempotency_key: Uuid,
}

impl Validate for ScanInput {
    fn validate(&self) -> Result<(), String> {
        if !within(&self.seed, 1, 250) {
            return Err("Seed : 1 à 250 caractères".into());
        }
        if !(1..=50).contains(&self.max_requests) {
            return Err("Budget : 1 à 50 requêtes".into());
        }
        Ok(())
    }
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
    ValidJson(data): ValidJson<ScanInput>,
) -> ApiResult<(StatusCode, Json<Scan>)> {
    let seed = parse_seed(&data.seed).map_err(|error| ApiError::unprocessable(error.to_string()))?;
    let mut tx = state.pool.begin().await?;
    require_access(&mut tx, investigation_id, user.id, Access::Write).await?;
    // Serialize submissions per investigation so the pending cap holds under concurrency.
    sqlx::query("SELECT id FROM investigations WHERE id = $1 FOR UPDATE")
        .bind(investigation_id)
        .execute(&mut *tx)
        .await?;
    let existing: Option<Scan> =
        sqlx::query_as("SELECT * FROM scans WHERE investigation_id = $1 AND idempotency_key = $2")
            .bind(investigation_id)
            .bind(data.idempotency_key)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(existing) = existing {
        if existing.seed != seed.path || existing.max_requests != data.max_requests {
            return Err(ApiError::conflict("Clé d'idempotence déjà utilisée pour un autre scan"));
        }
        return Ok((StatusCode::ACCEPTED, Json(existing)));
    }
    let pending: i64 =
        sqlx::query_scalar("SELECT count(*) FROM scans WHERE investigation_id = $1 AND status = ANY($2)")
            .bind(investigation_id)
            .bind(&PENDING_STATUSES[..])
            .fetch_one(&mut *tx)
            .await?;
    if pending >= MAX_PENDING_SCANS {
        return Err(ApiError::too_many("Trois scans sont déjà en attente ou actifs"));
    }
    let scan: Scan = sqlx::query_as(
        "INSERT INTO scans(id, investigation_id, requested_by, source, seed, status, max_requests, idempotency_key) \
         VALUES($1, $2, $3, 'github', $4, 'queued', $5, $6) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(investigation_id)
    .bind(user.id)
    .bind(&seed.path)
    .bind(data.max_requests)
    .bind(data.idempotency_key)
    .fetch_one(&mut *tx)
    .await?;
    audit(&mut tx, user.id, "scan.create", Some(investigation_id)).await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(scan)))
}

pub async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(scan_id): ApiPath<Uuid>,
) -> ApiResult<Json<Value>> {
    let mut tx = state.pool.begin().await?;
    let scan = accessible_scan(&mut tx, scan_id, user.id, Access::Write).await?;
    let cancelled = sqlx::query(
        "UPDATE scans SET status = 'cancelled', generation = generation + 1, finished_at = now() \
         WHERE id = $1 AND status = ANY($2)",
    )
    .bind(scan_id)
    .bind(&PENDING_STATUSES[..])
    .execute(&mut *tx)
    .await?;
    if cancelled.rows_affected() > 0 {
        audit(&mut tx, user.id, "scan.cancel", Some(scan.investigation_id)).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"status": "ok"})))
}
