//! Découverte GitHub : aperçu du plan de requêtes, puis lancement.
//!
//! L'aperçu n'appelle jamais GitHub. Une découverte est un scan de source
//! `search` : même file, mêmes droits, même plafond de scans en attente.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use observatory_domain::discovery::{self, ListRef, Options, Plan, SearchKind};
use observatory_domain::rules::Definition;
use observatory_domain::text::within;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use super::AppState;
use super::extract::{ApiPath, CurrentUser, ValidJson, Validate};
use super::scans::MAX_PENDING_SCANS;
use crate::error::{ApiError, ApiResult};
use crate::model::{PENDING_STATUSES, Scan};
use crate::security::{Access, audit, require_access};

/// Budget proposé par défaut pour une découverte.
const DEFAULT_REQUESTS: i32 = 12;

fn yes() -> bool {
    true
}

fn two() -> u32 {
    2
}

fn four() -> u32 {
    4
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ListChoice {
    id: Uuid,
    version: i32,
}

#[derive(Debug, Deserialize)]
pub struct DiscoveryInput {
    kind: SearchKind,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    list: Option<ListChoice>,
    #[serde(default = "yes")]
    include_forks: bool,
    #[serde(default = "two")]
    max_pages: u32,
    #[serde(default = "four")]
    max_subdivisions: u32,
    #[serde(default)]
    max_requests: Option<i32>,
    #[serde(default = "Uuid::new_v4")]
    idempotency_key: Uuid,
}

impl Validate for DiscoveryInput {
    fn validate(&self) -> Result<(), String> {
        match (&self.query, &self.list) {
            (Some(query), None) if within(query, 1, 256) => {}
            (None, Some(_)) => {}
            (Some(_), None) => return Err("Requête : 1 à 256 caractères".into()),
            _ => return Err("Une requête saisie ou une liste publiée, l'une ou l'autre".into()),
        }
        if self.max_requests.is_some_and(|budget| !(1..=50).contains(&budget)) {
            return Err("Budget : 1 à 50 requêtes".into());
        }
        Ok(())
    }
}

/// Compile le plan ; une liste doit appartenir au dossier.
async fn compile(conn: &mut PgConnection, investigation_id: Uuid, input: &DiscoveryInput) -> ApiResult<Plan> {
    let options = Options {
        kind: input.kind,
        include_forks: input.include_forks,
        max_pages: input.max_pages,
        max_subdivisions: input.max_subdivisions,
    };
    let plan = match (&input.query, input.list) {
        (Some(query), _) => discovery::from_query(query, options),
        (None, Some(choice)) => {
            let row: Option<(String, Value)> = sqlx::query_as(
                "SELECT l.name, v.definition FROM keyword_lists l \
                 JOIN keyword_list_versions v ON v.list_id = l.id \
                 WHERE l.id = $1 AND l.investigation_id = $2 AND v.version = $3",
            )
            .bind(choice.id)
            .bind(investigation_id)
            .bind(choice.version)
            .fetch_optional(&mut *conn)
            .await?;
            let Some((name, stored)) = row else {
                return Err(ApiError::not_found("Liste introuvable dans ce dossier"));
            };
            let definition = Definition::parse(&stored)
                .map_err(|error| ApiError::unprocessable(format!("Liste illisible : {error}")))?;
            let list = ListRef { id: choice.id.to_string(), version: choice.version, name };
            discovery::from_list(&definition, list, options)
        }
        (None, None) => Err("Une requête saisie ou une liste publiée".into()),
    };
    plan.map_err(ApiError::unprocessable)
}

fn describe(plan: &Plan) -> Value {
    json!({
        "plan": plan,
        "label": plan.label(),
        "max_requests_upper_bound": plan.max_requests(),
        "resource": plan.kind.resource(),
    })
}

/// Plan de requêtes et limites, sans appel à GitHub.
pub async fn preview(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
    ValidJson(data): ValidJson<DiscoveryInput>,
) -> ApiResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    require_access(&mut conn, investigation_id, user.id, Access::Read).await?;
    let plan = compile(&mut conn, investigation_id, &data).await?;
    Ok(Json(describe(&plan)))
}

/// Lance une découverte : un scan de source `search` portant son plan.
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
    ValidJson(data): ValidJson<DiscoveryInput>,
) -> ApiResult<(StatusCode, Json<Scan>)> {
    let mut tx = state.pool.begin().await?;
    require_access(&mut tx, investigation_id, user.id, Access::Write).await?;
    let plan = compile(&mut tx, investigation_id, &data).await?;
    let budget = data.max_requests.unwrap_or(DEFAULT_REQUESTS);
    let label = plan.label();
    let stored = serde_json::to_value(&plan).map_err(|error| ApiError::Internal(error.into()))?;
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
        if existing.source != "search"
            || existing.plan.as_ref() != Some(&stored)
            || existing.max_requests != budget
        {
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
        return Err(ApiError::too_many("Dix scans au plus en attente ou actifs par dossier"));
    }
    let scan: Scan = sqlx::query_as(
        "INSERT INTO scans(id, investigation_id, requested_by, source, seed, status, max_requests, idempotency_key, plan) \
         VALUES($1, $2, $3, 'search', $4, 'queued', $5, $6, $7) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(investigation_id)
    .bind(user.id)
    .bind(&label)
    .bind(budget)
    .bind(data.idempotency_key)
    .bind(&stored)
    .fetch_one(&mut *tx)
    .await?;
    audit(&mut tx, user.id, "discovery.create", Some(investigation_id)).await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(scan)))
}
