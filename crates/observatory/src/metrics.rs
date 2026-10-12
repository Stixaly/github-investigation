//! Mesures de référence (lot 0 du plan de collecte) : volumes, croissance,
//! durées et réutilisation, au format JSON pour comparer deux relevés.
//!
//! Les comptes des tables d'observation sont exacts ; les autres tailles
//! viennent du catalogue PostgreSQL. La croissance est datée par
//! l'enregistrement (`recorded_at`), jamais par une date d'observation inventée.

use serde_json::{Value, json};
use sqlx::PgPool;

/// Tables comptées exactement.
const COUNTED: &[&str] = &[
    "entities",
    "relations",
    "evidence",
    "scan_entities",
    "scan_relations",
    "http_cache",
    "scans",
    "keyword_list_versions",
    "rule_matches",
    "triage_items",
];
/// Jours de croissance relevés.
const DAYS: i32 = 14;

#[derive(Debug, sqlx::FromRow)]
struct Table {
    name: String,
    total_bytes: i64,
    index_bytes: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct Daily {
    day: String,
    evidence: i64,
    entity_observations: i64,
    relation_observations: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct Durations {
    finished: i64,
    p50: Option<f64>,
    p95: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct Reuse {
    requests: Option<i64>,
    not_modified: Option<i64>,
    cache_hits: Option<i64>,
    retries: Option<i64>,
    bytes: Option<i64>,
    requests_charged: Option<i64>,
}

pub async fn report(pool: &PgPool) -> sqlx::Result<Value> {
    let database_bytes: i64 =
        sqlx::query_scalar("SELECT pg_database_size(current_database())").fetch_one(pool).await?;
    let tables: Vec<Table> = sqlx::query_as(
        "SELECT c.relname AS name, pg_total_relation_size(c.oid) AS total_bytes, pg_indexes_size(c.oid) AS index_bytes \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND c.relkind = 'r' ORDER BY total_bytes DESC, name",
    )
    .fetch_all(pool)
    .await?;
    let mut counts = serde_json::Map::new();
    for table in COUNTED {
        // Only names from the constant list above are interpolated.
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(pool)
            .await?;
        counts.insert((*table).to_owned(), count.into());
    }
    let days: Vec<Daily> = sqlx::query_as(
        "SELECT to_char(d, 'YYYY-MM-DD') AS day, \
         (SELECT count(*) FROM evidence WHERE recorded_at >= d AND recorded_at < d + interval '1 day') AS evidence, \
         (SELECT count(*) FROM scan_entities WHERE recorded_at >= d AND recorded_at < d + interval '1 day') \
           AS entity_observations, \
         (SELECT count(*) FROM scan_relations WHERE recorded_at >= d AND recorded_at < d + interval '1 day') \
           AS relation_observations \
         FROM generate_series(date_trunc('day', now()) - make_interval(days => $1 - 1), date_trunc('day', now()), \
           interval '1 day') AS d ORDER BY d",
    )
    .bind(DAYS)
    .fetch_all(pool)
    .await?;
    let undated: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM scan_entities WHERE observed_at IS NULL) \
         + (SELECT count(*) FROM scan_relations WHERE observed_at IS NULL)",
    )
    .fetch_one(pool)
    .await?;
    let statuses: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, count(*) FROM scans GROUP BY status ORDER BY status")
            .fetch_all(pool)
            .await?;
    let durations: Durations = sqlx::query_as(
        "SELECT count(*) AS finished, \
         percentile_cont(0.5) WITHIN GROUP (ORDER BY extract(epoch FROM finished_at - started_at))::float8 AS p50, \
         percentile_cont(0.95) WITHIN GROUP (ORDER BY extract(epoch FROM finished_at - started_at))::float8 AS p95 \
         FROM scans WHERE source IN ('github', 'search') AND started_at IS NOT NULL AND finished_at IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    let reuse: Reuse = sqlx::query_as(
        "SELECT sum((coverage->'stats'->>'requests')::int8)::int8 AS requests, \
         sum((coverage->'stats'->>'not_modified')::int8)::int8 AS not_modified, \
         sum((coverage->'stats'->>'cache_hits')::int8)::int8 AS cache_hits, \
         sum((coverage->'stats'->>'retries')::int8)::int8 AS retries, \
         sum((coverage->'stats'->>'bytes')::int8)::int8 AS bytes, \
         sum(requests_used)::int8 AS requests_charged \
         FROM scans WHERE source IN ('github', 'search')",
    )
    .fetch_one(pool)
    .await?;
    Ok(json!({
        "measured_at": chrono::Utc::now(),
        "database_bytes": database_bytes,
        "rows": counts,
        "tables": tables.iter().map(|table| json!({
            "name": table.name, "total_bytes": table.total_bytes, "index_bytes": table.index_bytes,
        })).collect::<Vec<_>>(),
        "recorded_per_day": days.iter().map(|day| json!({
            "day": day.day, "evidence": day.evidence,
            "entity_observations": day.entity_observations, "relation_observations": day.relation_observations,
        })).collect::<Vec<_>>(),
        "observations_without_date": undated,
        "scans": {
            "by_status": statuses.into_iter()
                .map(|(status, count)| (status, Value::from(count))).collect::<serde_json::Map<_, _>>(),
            "finished": durations.finished,
            "duration_seconds_p50": durations.p50,
            "duration_seconds_p95": durations.p95,
        },
        "github": {
            "requests_sent": reuse.requests.unwrap_or(0),
            "requests_charged": reuse.requests_charged.unwrap_or(0),
            "not_modified": reuse.not_modified.unwrap_or(0),
            "cache_hits": reuse.cache_hits.unwrap_or(0),
            "retries": reuse.retries.unwrap_or(0),
            "bytes_received": reuse.bytes.unwrap_or(0),
        },
    }))
}
