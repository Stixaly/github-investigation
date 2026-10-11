//! Graphe borné d'un scan ou d'un dossier, comparaison, recoupements,
//! recherche, voisins communs et preuves.
//!
//! Toute lecture passe d'abord par les droits de l'investigation : une preuve
//! ou un recoupement n'est jamais un raccourci vers un dossier inaccessible.

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use observatory_domain::text::escape_like;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use super::AppState;
use super::extract::{ApiPath, ApiQuery, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::security::{Access, accessible_scan, require_access};

/// Liens renvoyés au plus par vue.
const MAX_EDGES: usize = 2000;
/// Entités renvoyées au plus par une recherche.
const MAX_FOUND: usize = 50;

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

/// Graphe borné des observations d'un ensemble de scans : chaque entité et chaque
/// relation y apparaît avec sa dernière observation. Les seeds passent en tête.
async fn bounded_graph(
    conn: &mut PgConnection,
    scans: &[Uuid],
    seeds: &[String],
    query: &GraphQuery,
) -> ApiResult<(Vec<Node>, Vec<Edge>, bool)> {
    if !(1..=500).contains(&query.limit) || query.q.chars().count() > 120 {
        return Err(ApiError::unprocessable("limit : 1 à 500 ; q : 120 caractères au plus"));
    }
    // The seeds first, then accounts, then the induced subgraph of the selected entities.
    // Only the constant LABEL_SQL is interpolated; every value is a bound parameter.
    let ids: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "WITH observed AS ( \
           SELECT e.id, e.kind, {LABEL_SQL} AS label \
           FROM scan_entities se JOIN entities e ON e.id = se.entity_id \
           JOIN evidence ev ON ev.hash = se.evidence_hash \
           CROSS JOIN LATERAL (SELECT coalesce(se.data, ev.payload) AS d) observation \
           WHERE se.scan_id = ANY($1)) \
         SELECT id FROM observed \
         WHERE ($2 = '' OR label ILIKE $3 ESCAPE '\\') \
           AND ($4::uuid IS NULL OR id = $4 OR id IN ( \
             SELECT CASE WHEN r.source_id = $4 THEN r.target_id ELSE r.source_id END \
             FROM relations r JOIN scan_relations sr ON sr.relation_id = r.id \
             WHERE sr.scan_id = ANY($1) AND (r.source_id = $4 OR r.target_id = $4) LIMIT 1000)) \
         GROUP BY id, kind \
         ORDER BY bool_or(label = ANY($5)) DESC, (kind = 'Account') DESC, id \
         LIMIT $6"
    )))
    .bind(scans)
    .bind(&query.q)
    .bind(format!("%{}%", escape_like(&query.q)))
    .bind(query.focus)
    .bind(seeds)
    .bind(query.limit + 1)
    .fetch_all(&mut *conn)
    .await?;
    let limit = usize::try_from(query.limit).unwrap_or(0);
    let selected = &ids[..ids.len().min(limit)];
    let mut nodes: Vec<Node> = sqlx::query_as(
        "SELECT DISTINCT ON (e.id) e.id, e.kind, e.external_id, e.provisional, \
         coalesce(se.data, ev.payload) AS data, se.observed_at, se.time_precision, se.evidence_hash, se.recorded_at \
         FROM entities e JOIN scan_entities se ON se.entity_id = e.id JOIN evidence ev ON ev.hash = se.evidence_hash \
         WHERE se.scan_id = ANY($1) AND e.id = ANY($2) ORDER BY e.id, se.recorded_at DESC, se.evidence_hash",
    )
    .bind(scans)
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
         WHERE sr.scan_id = ANY($1) AND r.source_id = ANY($2) AND r.target_id = ANY($2) \
         ORDER BY r.id, sr.recorded_at DESC LIMIT $3",
    )
    .bind(scans)
    .bind(selected)
    .bind(i64::try_from(MAX_EDGES).unwrap_or(i64::MAX) + 1)
    .fetch_all(&mut *conn)
    .await?;
    let truncated = ids.len() > limit || edges.len() > MAX_EDGES;
    edges.truncate(MAX_EDGES);
    Ok((nodes, edges, truncated))
}

pub async fn graph(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(scan_id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<GraphQuery>,
) -> ApiResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    let scan = accessible_scan(&mut conn, scan_id, user.id, Access::Read).await?;
    let (nodes, edges, truncated) =
        bounded_graph(&mut conn, &[scan_id], std::slice::from_ref(&scan.seed), &query).await?;
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

/// Graphe cumulé de tous les scans d'un dossier.
pub async fn investigation_graph(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<GraphQuery>,
) -> ApiResult<Json<Value>> {
    let mut conn = state.pool.acquire().await?;
    require_access(&mut conn, investigation_id, user.id, Access::Read).await?;
    let scans: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, seed FROM scans WHERE investigation_id = $1")
        .bind(investigation_id)
        .fetch_all(&mut *conn)
        .await?;
    let (ids, seeds): (Vec<Uuid>, Vec<String>) = scans.into_iter().unzip();
    let (nodes, edges, truncated) = bounded_graph(&mut conn, &ids, &seeds, &query).await?;
    Ok(Json(json!({
        "nodes": nodes,
        "edges": edges,
        "investigation_id": investigation_id,
        "scan_count": ids.len(),
        "temporal_mode": "investigation",
        "truncated": truncated,
        "notice": "Vue cumulée des scans du dossier : chaque élément est montré avec sa dernière observation. \
                   Une absence ne signifie pas une suppression.",
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

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
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

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
}

#[derive(Debug, sqlx::FromRow)]
struct Match {
    id: Uuid,
    kind: String,
    label: String,
    investigation_id: Uuid,
    title: String,
}

/// Entité trouvée et dossiers accessibles qui l'ont observée.
#[derive(Debug, Serialize)]
pub struct Found {
    id: Uuid,
    kind: String,
    label: String,
    investigations: Vec<Overlap>,
}

/// Recherche par fragment de nom dans les entités observées par les dossiers de l'analyste,
/// les plus ressemblantes d'abord.
pub async fn search(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiQuery(query): ApiQuery<SearchQuery>,
) -> ApiResult<Json<Vec<Found>>> {
    let q = query.q.trim();
    if !(2..=120).contains(&q.chars().count()) {
        return Err(ApiError::unprocessable("q : 2 à 120 caractères"));
    }
    let rows: Vec<Match> = sqlx::query_as(
        "SELECT e.id, e.kind, e.label, i.id AS investigation_id, i.title FROM memberships m \
         JOIN investigations i ON i.id = m.investigation_id \
         JOIN scans s ON s.investigation_id = i.id \
         JOIN scan_entities se ON se.scan_id = s.id \
         JOIN entities e ON e.id = se.entity_id \
         WHERE m.user_id = $1 AND e.label ILIKE $2 ESCAPE '\\' \
         GROUP BY e.id, e.kind, e.label, i.id, i.title \
         ORDER BY similarity(e.label, $3) DESC, e.label, i.title LIMIT 500",
    )
    .bind(user.id)
    .bind(format!("%{}%", escape_like(q)))
    .bind(q)
    .fetch_all(&state.pool)
    .await?;
    let mut found: Vec<Found> = Vec::new();
    for row in rows {
        let investigation = Overlap { id: row.investigation_id, title: row.title };
        if let Some(entity) = found.iter_mut().find(|entity| entity.id == row.id) {
            entity.investigations.push(investigation);
        } else if found.len() < MAX_FOUND {
            found.push(Found {
                id: row.id,
                kind: row.kind,
                label: row.label,
                investigations: vec![investigation],
            });
        }
    }
    Ok(Json(found))
}

#[derive(Debug, Deserialize)]
pub struct CommonQuery {
    a: Uuid,
    b: Uuid,
}

/// Entité reliée aux deux entités comparées, avec les types de relation de chaque côté.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Common {
    id: Uuid,
    kind: String,
    label: String,
    with_a: Vec<String>,
    with_b: Vec<String>,
}

/// Voisins communs de deux entités dans les observations d'un dossier.
pub async fn common(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(investigation_id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<CommonQuery>,
) -> ApiResult<Json<Value>> {
    if query.a == query.b {
        return Err(ApiError::unprocessable("Choisir deux entités différentes"));
    }
    let mut conn = state.pool.acquire().await?;
    require_access(&mut conn, investigation_id, user.id, Access::Read).await?;
    let rows: Vec<Common> = sqlx::query_as(
        "WITH links AS ( \
           SELECT DISTINCT r.source_id, r.kind, r.target_id FROM relations r \
           JOIN scan_relations sr ON sr.relation_id = r.id JOIN scans s ON s.id = sr.scan_id \
           WHERE s.investigation_id = $1 AND (r.source_id IN ($2, $3) OR r.target_id IN ($2, $3))), \
         ends AS ( \
           SELECT CASE WHEN source_id IN ($2, $3) THEN source_id ELSE target_id END AS anchor, \
                  CASE WHEN source_id IN ($2, $3) THEN target_id ELSE source_id END AS other, kind \
           FROM links) \
         SELECT e.id, e.kind, e.label, \
           array_agg(DISTINCT ends.kind) FILTER (WHERE anchor = $2) AS with_a, \
           array_agg(DISTINCT ends.kind) FILTER (WHERE anchor = $3) AS with_b \
         FROM ends JOIN entities e ON e.id = ends.other \
         WHERE ends.other NOT IN ($2, $3) \
         GROUP BY e.id, e.kind, e.label HAVING count(DISTINCT anchor) = 2 \
         ORDER BY e.kind, e.label LIMIT 200",
    )
    .bind(investigation_id)
    .bind(query.a)
    .bind(query.b)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Json(json!({
        "a": query.a,
        "b": query.b,
        "common": rows,
        "notice": "Relations observées dans ce dossier ; un voisin commun n'établit pas un lien entre les deux entités.",
    })))
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
