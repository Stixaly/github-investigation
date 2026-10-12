//! Évaluations des listes sur un dossier et file de triage.
//!
//! Une correspondance est une priorité de revue, jamais une qualification :
//! la file garde la preuve, l'extrait, la version de liste et de moteur, et la
//! décision de l'analyste.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use observatory_domain::rules::ENGINE_VERSION;
use observatory_domain::text::within;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::AppState;
use super::extract::{ApiPath, ApiQuery, CurrentUser, ValidJson, Validate};
use crate::error::{ApiError, ApiResult};
use crate::security::{Access, audit, require_access};

/// Tâches de triage par page.
const PAGE: i64 = 50;
const STATES: [&str; 4] = ["new", "in_review", "retained", "dismissed"];

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RunSummary {
    id: Uuid,
    list_id: Uuid,
    list_name: String,
    list_version: i32,
    engine_version: i32,
    status: String,
    counts: Value,
    error: Option<String>,
    requested_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

const RUN_COLUMNS: &str = "r.id, r.list_id, l.name AS list_name, r.list_version, r.engine_version, r.status, \
     r.counts, r.error, r.requested_at, r.finished_at";

/// Lance l'évaluation d'une version publiée sur les observations de son dossier.
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath((list, version)): ApiPath<(Uuid, i32)>,
) -> ApiResult<(StatusCode, Json<RunSummary>)> {
    let mut tx = state.pool.begin().await?;
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT investigation_id, name FROM keyword_lists WHERE id = $1 FOR UPDATE")
            .bind(list)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((investigation, _)) = row else {
        return Err(ApiError::not_found("Liste introuvable"));
    };
    require_access(&mut tx, investigation, user.id, Access::Write).await?;
    let published: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM keyword_list_versions WHERE list_id = $1 AND version = $2)",
    )
    .bind(list)
    .bind(version)
    .fetch_one(&mut *tx)
    .await?;
    if !published {
        return Err(ApiError::not_found("Version introuvable"));
    }
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM rule_runs WHERE list_id = $1 AND status IN ('queued', 'running'))",
    )
    .bind(list)
    .fetch_one(&mut *tx)
    .await?;
    if pending {
        return Err(ApiError::conflict("Une évaluation de cette liste est déjà en cours"));
    }
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rule_runs(id, investigation_id, list_id, list_version, engine_version, requested_by, status) \
         VALUES($1, $2, $3, $4, $5, $6, 'queued')",
    )
    .bind(id)
    .bind(investigation)
    .bind(list)
    .bind(version)
    .bind(i32::try_from(ENGINE_VERSION).unwrap_or(i32::MAX))
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    audit(&mut tx, user.id, "rule_run.create", Some(investigation)).await?;
    let run: RunSummary = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {RUN_COLUMNS} FROM rule_runs r JOIN keyword_lists l ON l.id = r.list_id WHERE r.id = $1"
    )))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(run)))
}

/// Dernières évaluations du dossier.
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<RunSummary>>> {
    let mut conn = state.pool.acquire().await?;
    require_access(&mut conn, investigation, user.id, Access::Read).await?;
    // Only the constant column list is interpolated.
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {RUN_COLUMNS} FROM rule_runs r JOIN keyword_lists l ON l.id = r.list_id \
         WHERE r.investigation_id = $1 ORDER BY r.requested_at DESC LIMIT 50"
    )))
    .bind(investigation)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Json(rows))
}

pub async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(run): ApiPath<Uuid>,
) -> ApiResult<Json<Value>> {
    let mut tx = state.pool.begin().await?;
    let investigation: Option<Uuid> =
        sqlx::query_scalar("SELECT investigation_id FROM rule_runs WHERE id = $1")
            .bind(run)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(investigation) = investigation else {
        return Err(ApiError::not_found("Évaluation introuvable"));
    };
    require_access(&mut tx, investigation, user.id, Access::Write).await?;
    sqlx::query(
        "UPDATE rule_runs SET status = 'cancelled', generation = generation + 1, finished_at = now(), lease_until = NULL \
         WHERE id = $1 AND status IN ('queued', 'running')",
    )
    .bind(run)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"status": "ok"})))
}

#[derive(Debug, Deserialize)]
pub struct TriageQuery {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    list: Option<Uuid>,
    #[serde(default = "first")]
    page: i64,
}

fn first() -> i64 {
    1
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TriageItem {
    id: Uuid,
    state: String,
    note: String,
    entity_id: Uuid,
    entity_kind: String,
    entity_label: String,
    list_id: Uuid,
    list_name: String,
    list_version: i32,
    engine_version: i32,
    rule_id: String,
    review_priority: String,
    hits: Value,
    exclusions: Value,
    not_evaluated: Value,
    evidence_hash: String,
    observed_at: Option<DateTime<Utc>>,
    evaluated_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
    updated_by: Option<String>,
}

/// File de triage du dossier, page par page, avec ses explications et ses compteurs par état.
pub async fn queue(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<TriageQuery>,
) -> ApiResult<Json<Value>> {
    if query.state.as_deref().is_some_and(|wanted| !STATES.contains(&wanted))
        || !(1..=1000).contains(&query.page)
    {
        return Err(ApiError::unprocessable(
            "state : new, in_review, retained ou dismissed ; page : 1 à 1000",
        ));
    }
    let mut conn = state.pool.acquire().await?;
    require_access(&mut conn, investigation, user.id, Access::Read).await?;
    let items: Vec<TriageItem> = sqlx::query_as(
        "SELECT t.id, t.state, t.note, e.id AS entity_id, e.kind AS entity_kind, e.label AS entity_label, \
         t.list_id, l.name AS list_name, r.list_version, r.engine_version, t.rule_id, m.review_priority, m.hits, \
         m.exclusions, m.not_evaluated, t.evidence_hash, m.observed_at, r.finished_at AS evaluated_at, t.updated_at, \
         u.login AS updated_by \
         FROM triage_items t JOIN entities e ON e.id = t.entity_id JOIN keyword_lists l ON l.id = t.list_id \
         JOIN rule_runs r ON r.id = t.last_run_id \
         JOIN rule_matches m ON m.run_id = t.last_run_id AND m.entity_id = t.entity_id AND m.rule_id = t.rule_id \
         LEFT JOIN users u ON u.id = t.updated_by \
         WHERE t.investigation_id = $1 AND ($2::text IS NULL OR t.state = $2) AND ($3::uuid IS NULL OR t.list_id = $3) \
         ORDER BY (m.review_priority = 'high') DESC, t.updated_at DESC, t.id LIMIT $4 OFFSET $5",
    )
    .bind(investigation)
    .bind(&query.state)
    .bind(query.list)
    .bind(PAGE)
    .bind((query.page - 1) * PAGE)
    .fetch_all(&mut *conn)
    .await?;
    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT state, count(*) FROM triage_items WHERE investigation_id = $1 AND ($2::uuid IS NULL OR list_id = $2) \
         GROUP BY state",
    )
    .bind(investigation)
    .bind(query.list)
    .fetch_all(&mut *conn)
    .await?;
    let counts: Map<String, Value> = STATES
        .iter()
        .map(|state| ((*state).to_owned(), 0.into()))
        .chain(counts.into_iter().map(|(state, count)| (state, count.into())))
        .collect();
    Ok(Json(json!({
        "items": items,
        "counts": counts,
        "page": query.page,
        "page_size": PAGE,
        "notice": "Une correspondance indique un sujet à examiner, jamais un dépôt ou un auteur malveillant.",
    })))
}

#[derive(Debug, Deserialize)]
pub struct DecisionInput {
    state: String,
    #[serde(default)]
    note: Option<String>,
}

impl Validate for DecisionInput {
    fn validate(&self) -> Result<(), String> {
        if !STATES.contains(&self.state.as_str()) {
            return Err("État : new, in_review, retained ou dismissed".into());
        }
        if self.note.as_deref().is_some_and(|note| !within(note, 0, 2000)) {
            return Err("Note : 2000 caractères au plus".into());
        }
        Ok(())
    }
}

/// Décision de l'analyste sur une tâche de triage.
pub async fn decide(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(item): ApiPath<Uuid>,
    ValidJson(data): ValidJson<DecisionInput>,
) -> ApiResult<Json<Value>> {
    let mut tx = state.pool.begin().await?;
    let investigation: Option<Uuid> =
        sqlx::query_scalar("SELECT investigation_id FROM triage_items WHERE id = $1 FOR UPDATE")
            .bind(item)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(investigation) = investigation else {
        return Err(ApiError::not_found("Tâche introuvable"));
    };
    require_access(&mut tx, investigation, user.id, Access::Write).await?;
    sqlx::query(
        "UPDATE triage_items SET state = $1, note = coalesce($2, note), updated_by = $3, updated_at = now() WHERE id = $4",
    )
    .bind(&data.state)
    .bind(&data.note)
    .bind(user.id)
    .bind(item)
    .execute(&mut *tx)
    .await?;
    audit(&mut tx, user.id, "triage.update", Some(investigation)).await?;
    tx.commit().await?;
    Ok(Json(json!({"id": item, "state": data.state})))
}
