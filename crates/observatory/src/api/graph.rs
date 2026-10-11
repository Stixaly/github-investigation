//! Graphe borné d'un scan, comparaison, recoupements et preuves.
//!
//! Toute lecture passe d'abord par les droits de l'investigation : une preuve
//! ou un recoupement n'est jamais un raccourci vers un dossier inaccessible.

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use observatory_domain::text::escape_like;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::AppState;
use super::extract::{ApiPath, ApiQuery, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::security::{Access, accessible_scan};

/// Liens renvoyés au plus par vue.
const MAX_EDGES: usize = 2000;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Node {
    id: Uuid,
    kind: String,
    external_id: String,
    provisional: bool,
    data: Value,
    observed_at: Option<DateTime<Utc>>,
    time_precision: String,
    evidence_hash: String,
    recorded_at: DateTime<Utc>,
    #[sqlx(skip)]
    label: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Edge {
    id: Uuid,
    source_id: Uuid,
    kind: String,
    target_id: Uuid,
    context: String,
    evidence_hash: String,
    observed_at: Option<DateTime<Utc>>,
    source_event_at: Option<DateTime<Utc>>,
    time_precision: String,
}

fn default_limit() -> i64 {
    300
}

#[derive(Debug, Deserialize)]
pub struct GraphQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    q: String,
    focus: Option<Uuid>,
}

/// Libellé d'une entité observée : nom complet, login, titre, sinon identifiant.
fn label(data: &Value, external_id: &str) -> String {
    ["full_name", "login", "title"]
        .iter()
        .find_map(|key| data.get(key).and_then(Value::as_str).filter(|value| !value.is_empty()))
        .unwrap_or(external_id)
        .to_owned()
}

/// Same expression as `label`, in SQL; `d` is the observed object.
const LABEL_SQL: &str =
    "coalesce(nullif(d->>'full_name', ''), nullif(d->>'login', ''), nullif(d->>'title', ''), e.external_id)";

pub async fn graph(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(scan_id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<GraphQuery>,
) -> ApiResult<Json<Value>> {
    if !(1..=500).contains(&query.limit) || query.q.chars().count() > 120 {
        return Err(ApiError::unprocessable("limit : 1 à 500 ; q : 120 caractères au plus"));
    }
    let mut conn = state.pool.acquire().await?;
    let scan = accessible_scan(&mut conn, scan_id, user.id, Access::Read).await?;
    // The seed first, then accounts, then the induced subgraph of the selected entities.
    let ids: Vec<Uuid> = sqlx::query_scalar(&format!(
        "WITH observed AS ( \
           SELECT e.id, e.kind, {LABEL_SQL} AS label \
           FROM scan_entities se JOIN entities e ON e.id = se.entity_id \
           JOIN evidence ev ON ev.hash = se.evidence_hash \
           CROSS JOIN LATERAL (SELECT coalesce(se.data, ev.payload) AS d) observation \
           WHERE se.scan_id = $1) \
         SELECT id FROM observed \
         WHERE ($2 = '' OR label ILIKE $3 ESCAPE '\\') \
           AND ($4::uuid IS NULL OR id = $4 OR id IN ( \
             SELECT CASE WHEN r.source_id = $4 THEN r.target_id ELSE r.source_id END \
             FROM relations r JOIN scan_relations sr ON sr.relation_id = r.id \
             WHERE sr.scan_id = $1 AND (r.source_id = $4 OR r.target_id = $4) LIMIT 1000)) \
         GROUP BY id, kind \
         ORDER BY bool_or(label = $5) DESC, (kind = 'Account') DESC, id \
         LIMIT $6"
    ))
    .bind(scan_id)
    .bind(&query.q)
    .bind(format!("%{}%", escape_like(&query.q)))
    .bind(query.focus)
    .bind(&scan.seed)
    .bind(query.limit + 1)
    .fetch_all(&mut *conn)
    .await?;
    let limit = usize::try_from(query.limit).unwrap_or(0);
    let selected = &ids[..ids.len().min(limit)];
    let mut nodes: Vec<Node> = sqlx::query_as(
        "SELECT DISTINCT ON (e.id) e.id, e.kind, e.external_id, e.provisional, \
         coalesce(se.data, ev.payload) AS data, se.observed_at, se.time_precision, se.evidence_hash, se.recorded_at \
         FROM entities e JOIN scan_entities se ON se.entity_id = e.id JOIN evidence ev ON ev.hash = se.evidence_hash \
         WHERE se.scan_id = $1 AND e.id = ANY($2) ORDER BY e.id, se.recorded_at DESC, se.evidence_hash",
    )
    .bind(scan_id)
    .bind(selected)
    .fetch_all(&mut *conn)
    .await?;
    for node in &mut nodes {
        node.label = label(&node.data, &node.external_id);
    }
    let mut edges: Vec<Edge> = sqlx::query_as(
        "SELECT DISTINCT ON (r.id) r.id, r.source_id, r.kind, r.target_id, r.context, sr.evidence_hash, \
         sr.observed_at, sr.source_event_at, sr.time_precision \
         FROM relations r JOIN scan_relations sr ON sr.relation_id = r.id \
         WHERE sr.scan_id = $1 AND r.source_id = ANY($2) AND r.target_id = ANY($2) \
         ORDER BY r.id, sr.recorded_at DESC LIMIT $3",
    )
    .bind(scan_id)
    .bind(selected)
    .bind(i64::try_from(MAX_EDGES).unwrap_or(i64::MAX) + 1)
    .fetch_all(&mut *conn)
    .await?;
    let truncated = ids.len() > limit || edges.len() > MAX_EDGES;
    edges.truncate(MAX_EDGES);
    Ok(Json(json!({
        "nodes": nodes,
        "edges": edges,
        "coverage": scan.coverage,
        "scan_id": scan_id,
        "temporal_mode": "scan",
        "truncated": truncated,
        "notice": "Vue des observations de ce scan. Une absence ne signifie pas une suppression.",
    })))
}

#[derive(Debug, sqlx::FromRow)]
struct Counts {
    newly_observed: i64,
    not_reobserved: i64,
    shared: i64,
}

pub async fn compare(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath((scan_id, other_id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    let first = accessible_scan(&mut conn, scan_id, user.id, Access::Read).await?;
    let second = accessible_scan(&mut conn, other_id, user.id, Access::Read).await?;
    let counts: Counts = sqlx::query_as(
        "WITH a AS (SELECT DISTINCT relation_id FROM scan_relations WHERE scan_id = $1), \
         b AS (SELECT DISTINCT relation_id FROM scan_relations WHERE scan_id = $2) \
         SELECT count(*) FILTER (WHERE a.relation_id IS NULL) AS newly_observed, \
         count(*) FILTER (WHERE b.relation_id IS NULL) AS not_reobserved, \
         count(*) FILTER (WHERE a.relation_id IS NOT NULL AND b.relation_id IS NOT NULL) AS shared \
         FROM a FULL JOIN b USING (relation_id)",
    )
    .bind(scan_id)
    .bind(other_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Json(json!({
        "newly_observed": counts.newly_observed,
        "not_reobserved": counts.not_reobserved,
        "shared": counts.shared,
        "from": first.id,
        "to": second.id,
        "coverage_from": first.coverage,
        "coverage_to": second.coverage,
        "notice": "Non réobservé ne signifie pas supprimé. Comparer les couvertures.",
    })))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Overlap {
    id: Uuid,
    title: String,
}

/// Investigations accessibles à l'analyste où l'entité a aussi été observée.
pub async fn overlaps(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(entity_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<Overlap>>> {
    let rows = sqlx::query_as(
        "SELECT DISTINCT i.id, i.title FROM investigations i \
         JOIN memberships m ON m.investigation_id = i.id \
         JOIN scans s ON s.investigation_id = i.id JOIN scan_entities se ON se.scan_id = s.id \
         WHERE m.user_id = $1 AND se.entity_id = $2 ORDER BY i.title LIMIT 100",
    )
    .bind(user.id)
    .bind(entity_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Evidence {
    hash: String,
    payload: Value,
    source: String,
    recorded_at: DateTime<Utc>,
}

/// Preuve d'une entité ou d'une relation observée dans un scan accessible.
pub async fn evidence(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(digest): ApiPath<String>,
) -> ApiResult<Json<Evidence>> {
    let row: Option<Evidence> = sqlx::query_as(
        "SELECT e.hash, e.payload, e.source, e.recorded_at FROM evidence e WHERE e.hash = $1 AND EXISTS ( \
           SELECT 1 FROM scans s JOIN memberships m ON m.investigation_id = s.investigation_id AND m.user_id = $2 \
           WHERE s.id IN (SELECT scan_id FROM scan_entities WHERE evidence_hash = e.hash \
                          UNION SELECT scan_id FROM scan_relations WHERE evidence_hash = e.hash))",
    )
    .bind(&digest)
    .bind(user.id)
    .fetch_optional(&state.pool)
    .await?;
    row.map(Json).ok_or_else(|| ApiError::not_found("Preuve introuvable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_prefer_names() {
        assert_eq!(label(&json!({"full_name": "a/b", "login": "a"}), "1"), "a/b");
        assert_eq!(label(&json!({"login": "a", "title": "t"}), "1"), "a");
        assert_eq!(label(&json!({"title": "#1 Bug"}), "1"), "#1 Bug");
        assert_eq!(label(&json!({"full_name": ""}), "42"), "42");
    }
}
