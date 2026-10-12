//! Listes de termes d'un dossier : publication de versions immuables, export
//! et essai sur un exemple. L'évaluation sur le corpus viendra avec le triage.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use observatory_domain::rules::{Definition, ENGINE_VERSION, RuleResult, TestFailure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use super::AppState;
use super::extract::{ApiPath, CurrentUser, ValidJson, Validate};
use crate::error::{ApiError, ApiResult};
use crate::security::{Access, audit, require_access};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ListSummary {
    id: Uuid,
    name: String,
    version: i32,
    rules: i64,
    published_at: DateTime<Utc>,
    published_by: String,
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<ListSummary>>> {
    let mut conn = state.pool.acquire().await?;
    require_access(&mut conn, investigation_id, user.id, Access::Read).await?;
    let rows = sqlx::query_as(
        "SELECT DISTINCT ON (l.id) l.id, l.name, v.version, jsonb_array_length(v.definition->'rules')::int8 AS rules, \
         v.published_at, u.login AS published_by \
         FROM keyword_lists l JOIN keyword_list_versions v ON v.list_id = l.id JOIN users u ON u.id = v.published_by \
         WHERE l.investigation_id = $1 ORDER BY l.id, v.version DESC",
    )
    .bind(investigation_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut rows: Vec<ListSummary> = rows;
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct DefinitionInput {
    definition: Value,
}

impl Validate for DefinitionInput {
    fn validate(&self) -> Result<(), String> {
        if self.definition.is_object() { Ok(()) } else { Err("Liste : objet JSON attendu".into()) }
    }
}

/// Liste lisible et dont tous les tests intégrés réussissent ; 422 détaillé sinon.
fn publishable(value: &Value) -> ApiResult<Definition> {
    let definition = Definition::parse(value).map_err(|error| ApiError::unprocessable(error.to_string()))?;
    let failures = definition.failing_tests();
    if let Some(summary) = describe(&failures) {
        return Err(ApiError::unprocessable(summary));
    }
    Ok(definition)
}

fn describe(failures: &[TestFailure]) -> Option<String> {
    if failures.is_empty() {
        return None;
    }
    let label =
        |outcome| serde_json::to_value(outcome).ok().and_then(|value| value.as_str().map(str::to_owned));
    let details: Vec<String> = failures
        .iter()
        .map(|failure| {
            format!(
                "règle « {} », test {} : {} attendu, {} obtenu",
                failure.rule,
                failure.test + 1,
                label(failure.expected).unwrap_or_default(),
                label(failure.actual).unwrap_or_default()
            )
        })
        .collect();
    Some(format!("Tests de la liste en échec — {}", details.join(" ; ")))
}

async fn publish(
    conn: &mut PgConnection,
    list: Uuid,
    version: i32,
    definition: &Definition,
    user: Uuid,
) -> sqlx::Result<()> {
    let stored = serde_json::to_value(definition).map_err(|error| sqlx::Error::Encode(Box::new(error)))?;
    sqlx::query(
        "INSERT INTO keyword_list_versions(list_id, version, definition, engine_version, published_by) \
         VALUES($1, $2, $3, $4, $5)",
    )
    .bind(list)
    .bind(version)
    .bind(stored)
    .bind(i32::try_from(ENGINE_VERSION).unwrap_or(i32::MAX))
    .bind(user)
    .execute(conn)
    .await
    .map(|_| ())
}

/// Crée une liste et publie sa version 1.
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
    ValidJson(data): ValidJson<DefinitionInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let definition = publishable(&data.definition)?;
    let mut tx = state.pool.begin().await?;
    require_access(&mut tx, investigation_id, user.id, Access::Write).await?;
    let id = Uuid::new_v4();
    let created = sqlx::query(
        "INSERT INTO keyword_lists(id, investigation_id, name, created_by) VALUES($1, $2, $3, $4) \
         ON CONFLICT (investigation_id, name) DO NOTHING",
    )
    .bind(id)
    .bind(investigation_id)
    .bind(definition.name.trim())
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    if created.rows_affected() == 0 {
        return Err(ApiError::conflict("Une liste porte déjà ce nom : en publier une nouvelle version"));
    }
    publish(&mut tx, id, 1, &definition, user.id).await?;
    audit(&mut tx, user.id, "keyword_list.create", Some(investigation_id)).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "version": 1}))))
}

/// Dossier d'une liste, si l'analyste y a l'accès demandé.
async fn accessible_list(
    conn: &mut PgConnection,
    list: Uuid,
    user: Uuid,
    access: Access,
) -> ApiResult<(Uuid, String)> {
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT investigation_id, name FROM keyword_lists WHERE id = $1")
            .bind(list)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((investigation, name)) = row else {
        return Err(ApiError::not_found("Liste introuvable"));
    };
    require_access(conn, investigation, user, access).await?;
    Ok((investigation, name))
}

/// Publie une nouvelle version ; le nom d'une liste ne change pas.
pub async fn publish_version(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(list): ApiPath<Uuid>,
    ValidJson(data): ValidJson<DefinitionInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let definition = publishable(&data.definition)?;
    let mut tx = state.pool.begin().await?;
    let (investigation, name) = accessible_list(&mut tx, list, user.id, Access::Write).await?;
    if definition.name.trim() != name {
        return Err(ApiError::unprocessable(format!("Le nom d'une liste ne change pas : « {name} »")));
    }
    // The list row serializes concurrent publications.
    sqlx::query("SELECT id FROM keyword_lists WHERE id = $1 FOR UPDATE").bind(list).execute(&mut *tx).await?;
    let version: i32 = sqlx::query_scalar(
        "SELECT coalesce(max(version), 0) + 1 FROM keyword_list_versions WHERE list_id = $1",
    )
    .bind(list)
    .fetch_one(&mut *tx)
    .await?;
    publish(&mut tx, list, version, &definition, user.id).await?;
    audit(&mut tx, user.id, "keyword_list.publish", Some(investigation)).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id": list, "version": version}))))
}

#[derive(Debug, sqlx::FromRow)]
struct VersionRow {
    definition: Value,
    engine_version: i32,
    published_at: DateTime<Utc>,
    published_by: String,
}

/// Une version publiée, au format d'import : l'export se réimporte tel quel.
pub async fn version(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath((list, version)): ApiPath<(Uuid, i32)>,
) -> ApiResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    accessible_list(&mut conn, list, user.id, Access::Read).await?;
    let row: Option<VersionRow> = sqlx::query_as(
        "SELECT v.definition, v.engine_version, v.published_at, u.login AS published_by \
         FROM keyword_list_versions v JOIN users u ON u.id = v.published_by WHERE v.list_id = $1 AND v.version = $2",
    )
    .bind(list)
    .bind(version)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found("Version introuvable"));
    };
    let mut definition = row.definition;
    if let Some(object) = definition.as_object_mut() {
        object.insert("version".into(), version.into());
        object.insert("status".into(), "published".into());
    }
    Ok(Json(json!({
        "list_id": list,
        "version": version,
        "engine_version": row.engine_version,
        "published_at": row.published_at,
        "published_by": row.published_by,
        "definition": definition,
    })))
}

#[derive(Debug, Deserialize)]
pub struct PreviewInput {
    definition: Value,
    #[serde(default)]
    input: Map<String, Value>,
}

impl Validate for PreviewInput {
    fn validate(&self) -> Result<(), String> {
        if self.definition.is_object() { Ok(()) } else { Err("Liste : objet JSON attendu".into()) }
    }
}

#[derive(Debug, Serialize)]
pub struct Preview {
    engine_version: u32,
    failing_tests: Vec<TestFailure>,
    results: Vec<RuleResult>,
}

/// Valide une liste, exécute ses tests et l'évalue sur un exemple saisi, sans rien enregistrer.
pub async fn preview(
    _user: CurrentUser,
    ValidJson(data): ValidJson<PreviewInput>,
) -> ApiResult<Json<Preview>> {
    let definition =
        Definition::parse(&data.definition).map_err(|error| ApiError::unprocessable(error.to_string()))?;
    let invalid = data.input.iter().any(|(field, value)| {
        let text = matches!(value, Value::Null | Value::String(_))
            || value.as_array().is_some_and(|items| items.iter().all(Value::is_string));
        !observatory_domain::rules::FIELDS.contains(&field.as_str()) || !text
    });
    if invalid {
        return Err(ApiError::unprocessable(
            "Exemple : champs connus, valeurs texte, null ou listes de textes",
        ));
    }
    Ok(Json(Preview {
        engine_version: ENGINE_VERSION,
        failing_tests: definition.failing_tests(),
        results: definition.evaluate(&data.input),
    }))
}
