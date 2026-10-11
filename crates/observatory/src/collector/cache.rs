//! Réponses GitHub réutilisables, cloisonnées par identité d'accès et représentation.
//!
//! Le corps est gardé tel que sérialisé par serde_json (`float_roundtrip`) :
//! relu, il redonne exactement la même valeur, donc les mêmes empreintes de preuve.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

use super::http::REPRESENTATION;

/// Réponse en cache pour une URL et une identité d'accès.
#[derive(Debug, Clone, PartialEq)]
pub struct Cached {
    pub etag: Option<String>,
    pub payload: Value,
    pub link: String,
    /// Dernière confirmation par GitHub (réponse 200 ou 304).
    pub validated_at: DateTime<Utc>,
    /// Confirmée depuis moins que la fenêtre de fraîcheur : réutilisable sans requête.
    pub fresh: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    etag: Option<String>,
    body: String,
    link: String,
    validated_at: DateTime<Utc>,
    fresh: bool,
}

/// Cherche la réponse en cache ; une entrée illisible compte comme absente.
pub async fn lookup(
    pool: &PgPool,
    url: &str,
    identity: &str,
    freshness: Duration,
) -> sqlx::Result<Option<Cached>> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT etag, body, link, validated_at, \
         ($4::float8 > 0 AND validated_at > now() - make_interval(secs => $4::float8)) AS fresh \
         FROM http_cache WHERE url = $1 AND identity = $2 AND representation = $3",
    )
    .bind(url)
    .bind(identity)
    .bind(REPRESENTATION)
    .bind(freshness.as_secs_f64())
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|row| {
        let payload = serde_json::from_str(&row.body).ok()?;
        Some(Cached {
            etag: row.etag,
            payload,
            link: row.link,
            validated_at: row.validated_at,
            fresh: row.fresh,
        })
    }))
}

/// Enregistre une réponse 200, validée maintenant.
pub async fn store(
    pool: &PgPool,
    url: &str,
    identity: &str,
    etag: Option<&str>,
    payload: &Value,
    link: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO http_cache(url, identity, representation, etag, body, link) VALUES($1, $2, $3, $4, $5, $6) \
         ON CONFLICT(url, identity, representation) DO UPDATE SET etag = excluded.etag, body = excluded.body, \
         link = excluded.link, fetched_at = now(), validated_at = now()",
    )
    .bind(url)
    .bind(identity)
    .bind(REPRESENTATION)
    .bind(etag)
    .bind(payload.to_string())
    .bind(link)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Note qu'une réponse 304 vient de confirmer la version en cache.
pub async fn revalidate(pool: &PgPool, url: &str, identity: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE http_cache SET validated_at = now() WHERE url = $1 AND identity = $2 AND representation = $3",
    )
    .bind(url)
    .bind(identity)
    .bind(REPRESENTATION)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Supprime les réponses que GitHub n'a pas confirmées depuis `retention` ; retourne leur nombre.
pub async fn purge(pool: &PgPool, retention: Duration) -> sqlx::Result<u64> {
    sqlx::query("DELETE FROM http_cache WHERE validated_at < now() - make_interval(secs => $1)")
        .bind(retention.as_secs_f64())
        .execute(pool)
        .await
        .map(|done| done.rows_affected())
}
