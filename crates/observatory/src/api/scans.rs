//! Demande, liste et annulation des scans ; profils de collecte.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use observatory_domain::parse_seed;
use observatory_domain::profile::CollectionSettings;
use observatory_domain::text::within;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use super::AppState;
use super::extract::{ApiPath, CurrentUser, ValidJson, Validate};
use crate::error::{ApiError, ApiResult};
use crate::model::{PENDING_STATUSES, Scan};
use crate::security::{Access, accessible_scan, audit, require_access};

/// Scans en attente ou actifs tolérés par investigation.
pub const MAX_PENDING_SCANS: i64 = 10;
/// Seeds au plus dans un lancement groupé.
pub const MAX_SEEDS: usize = 10;
/// Profil appliqué quand la demande n'en nomme pas.
pub const DEFAULT_PROFILE: &str = "reseau";

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

/// Dernière version d'un profil de collecte.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Profile {
    name: String,
    version: i32,
    label: String,
    description: String,
    settings: Value,
    created_at: DateTime<Utc>,
}

/// Profils disponibles, dans leur dernière version.
pub async fn profiles(State(state): State<AppState>, _user: CurrentUser) -> ApiResult<Json<Vec<Profile>>> {
    let rows = sqlx::query_as(
        "SELECT DISTINCT ON (name) name, version, label, description, settings, created_at \
         FROM collection_profiles ORDER BY name, version DESC",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

/// Profil retenu pour un lancement, avec ses réglages validés.
#[derive(Debug)]
struct Resolved {
    name: String,
    version: i32,
    raw: Value,
    settings: CollectionSettings,
}

async fn resolve_profile(conn: &mut PgConnection, name: Option<&str>) -> ApiResult<Resolved> {
    let name = name.unwrap_or(DEFAULT_PROFILE);
    let row: Option<(i32, Value)> = sqlx::query_as(
        "SELECT version, settings FROM collection_profiles WHERE name = $1 ORDER BY version DESC LIMIT 1",
    )
    .bind(name)
    .fetch_optional(conn)
    .await?;
    let Some((version, raw)) = row else {
        return Err(ApiError::unprocessable(format!("Profil inconnu : {name}")));
    };
    let settings: CollectionSettings = serde_json::from_value(raw.clone())
        .map_err(|_| ApiError::unprocessable("Profil illisible : publier une version corrigée"))?;
    settings.validate().map_err(|reason| ApiError::unprocessable(format!("Profil invalide : {reason}")))?;
    Ok(Resolved { name: name.to_owned(), version, raw, settings })
}

fn valid_options(profile: Option<&String>, max_requests: Option<i32>) -> Result<(), String> {
    if profile.is_some_and(|name| !within(name, 1, 40)) {
        return Err("Profil : 1 à 40 caractères".into());
    }
    if max_requests.is_some_and(|budget| !(1..=50).contains(&budget)) {
        return Err("Budget : 1 à 50 requêtes".into());
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ScanInput {
    seed: String,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    max_requests: Option<i32>,
    #[serde(default = "Uuid::new_v4")]
    idempotency_key: Uuid,
}

impl Validate for ScanInput {
    fn validate(&self) -> Result<(), String> {
        if !within(&self.seed, 1, 250) {
            return Err("Seed : 1 à 250 caractères".into());
        }
        valid_options(self.profile.as_ref(), self.max_requests)
    }
}

#[derive(Debug, Deserialize)]
pub struct BatchInput {
    seeds: Vec<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    max_requests: Option<i32>,
    #[serde(default = "Uuid::new_v4")]
    idempotency_key: Uuid,
}

impl Validate for BatchInput {
    fn validate(&self) -> Result<(), String> {
        if self.seeds.is_empty() || self.seeds.len() > MAX_SEEDS {
            return Err(format!("Seeds : 1 à {MAX_SEEDS} par lancement"));
        }
        if self.seeds.iter().any(|seed| !within(seed, 1, 250)) {
            return Err("Seed : 1 à 250 caractères".into());
        }
        valid_options(self.profile.as_ref(), self.max_requests)
    }
}

/// Scan déjà créé avec cette clé d'idempotence ; 409 s'il diffère de la demande.
async fn replayed(
    conn: &mut PgConnection,
    investigation_id: Uuid,
    key: Uuid,
    seed: &str,
    profile: &Resolved,
    budget: i32,
) -> ApiResult<Option<Scan>> {
    let existing: Option<Scan> =
        sqlx::query_as("SELECT * FROM scans WHERE investigation_id = $1 AND idempotency_key = $2")
            .bind(investigation_id)
            .bind(key)
            .fetch_optional(conn)
            .await?;
    match existing {
        Some(scan)
            if scan.seed != seed
                || scan.max_requests != budget
                || scan.profile_name.as_deref() != Some(profile.name.as_str()) =>
        {
            Err(ApiError::conflict("Clé d'idempotence déjà utilisée pour un autre scan"))
        }
        other => Ok(other),
    }
}

async fn pending(conn: &mut PgConnection, investigation_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM scans WHERE investigation_id = $1 AND status = ANY($2)")
        .bind(investigation_id)
        .bind(&PENDING_STATUSES[..])
        .fetch_one(conn)
        .await
}

async fn insert(
    conn: &mut PgConnection,
    investigation_id: Uuid,
    user: Uuid,
    seed: &str,
    profile: &Resolved,
    budget: i32,
    key: Uuid,
) -> sqlx::Result<Scan> {
    sqlx::query_as(
        "INSERT INTO scans(id, investigation_id, requested_by, source, seed, status, max_requests, idempotency_key, \
         profile_name, profile_version, profile) \
         VALUES($1, $2, $3, 'github', $4, 'queued', $5, $6, $7, $8, $9) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(investigation_id)
    .bind(user)
    .bind(seed)
    .bind(budget)
    .bind(key)
    .bind(&profile.name)
    .bind(profile.version)
    .bind(&profile.raw)
    .fetch_one(conn)
    .await
}

/// Verrouille le dossier : le plafond de scans en attente tient sous concurrence.
async fn lock_investigation(conn: &mut PgConnection, investigation_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("SELECT id FROM investigations WHERE id = $1 FOR UPDATE")
        .bind(investigation_id)
        .execute(conn)
        .await
        .map(|_| ())
}

fn full() -> ApiError {
    ApiError::too_many("Dix scans au plus en attente ou actifs par dossier")
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
    let profile = resolve_profile(&mut tx, data.profile.as_deref()).await?;
    let budget = profile.settings.budget(data.max_requests).map_err(ApiError::unprocessable)?;
    lock_investigation(&mut tx, investigation_id).await?;
    if let Some(existing) =
        replayed(&mut tx, investigation_id, data.idempotency_key, &seed.path, &profile, budget).await?
    {
        return Ok((StatusCode::ACCEPTED, Json(existing)));
    }
    if pending(&mut tx, investigation_id).await? >= MAX_PENDING_SCANS {
        return Err(full());
    }
    let scan = insert(&mut tx, investigation_id, user.id, &seed.path, &profile, budget, data.idempotency_key)
        .await?;
    audit(&mut tx, user.id, "scan.create", Some(investigation_id)).await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(scan)))
}

/// Lance un scan par seed distincte, avec le même profil et le même budget.
///
/// Tout ou rien : si le plafond de scans en attente ne laisse pas la place pour
/// tous, aucun n'est créé. Rejouer la même demande renvoie les mêmes scans.
pub async fn create_batch(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
    ValidJson(data): ValidJson<BatchInput>,
) -> ApiResult<(StatusCode, Json<Vec<Scan>>)> {
    let mut seeds: Vec<String> = Vec::new();
    for raw in &data.seeds {
        let seed = parse_seed(raw).map_err(|error| ApiError::unprocessable(format!("{raw} : {error}")))?;
        if !seeds.iter().any(|known| known.eq_ignore_ascii_case(&seed.path)) {
            seeds.push(seed.path);
        }
    }
    let mut tx = state.pool.begin().await?;
    require_access(&mut tx, investigation_id, user.id, Access::Write).await?;
    let profile = resolve_profile(&mut tx, data.profile.as_deref()).await?;
    let budget = profile.settings.budget(data.max_requests).map_err(ApiError::unprocessable)?;
    lock_investigation(&mut tx, investigation_id).await?;
    let mut scans = Vec::with_capacity(seeds.len());
    let mut missing = Vec::new();
    for seed in &seeds {
        // One stable key per seed, derived from the request key.
        let key = Uuid::new_v5(&data.idempotency_key, seed.to_lowercase().as_bytes());
        let existing = replayed(&mut tx, investigation_id, key, seed, &profile, budget).await?;
        if existing.is_none() {
            missing.push((seed, key));
        }
        scans.push(existing);
    }
    let room = MAX_PENDING_SCANS - pending(&mut tx, investigation_id).await?;
    if i64::try_from(missing.len()).unwrap_or(i64::MAX) > room {
        return Err(full());
    }
    let mut created = Vec::with_capacity(missing.len());
    for (seed, key) in missing {
        created.push(insert(&mut tx, investigation_id, user.id, seed, &profile, budget, key).await?);
    }
    if !created.is_empty() {
        audit(&mut tx, user.id, "scan.create", Some(investigation_id)).await?;
    }
    tx.commit().await?;
    let mut created = created.into_iter();
    let scans = scans.into_iter().filter_map(|scan| scan.or_else(|| created.next())).collect();
    Ok((StatusCode::ACCEPTED, Json(scans)))
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
