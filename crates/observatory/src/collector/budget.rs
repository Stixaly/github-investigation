//! Rythme des requêtes GitHub, partagé par tous les workers à travers PostgreSQL.
//!
//! Chaque ressource de quota (`core` pour REST, `search`, `graphql`) a sa ligne
//! dans `github_rate_limits` : prochaine requête autorisée et dernier état
//! annoncé par GitHub, décrémenté à chaque réservation.

use std::fmt::Write as _;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::http::{GitHub, RateLimit};
use super::{CollectError, guard};
use crate::model::Scan;

/// Ressource de quota de l'API REST.
pub const CORE: &str = "core";

/// Fenêtre de quota en cours, telle que la base l'estime.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub remaining: i32,
    pub quota: i32,
    /// Secondes avant la remise à zéro.
    pub until_reset: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pace {
    /// Requête autorisée ; la suivante attendra ce nombre de secondes.
    Go(f64),
    /// Aucune requête avant ce nombre de secondes.
    Wait(f64),
}

#[derive(Debug, sqlx::FromRow)]
struct Limit {
    resource: String,
    remaining: Option<i32>,
    quota: Option<i32>,
    reset_at: Option<DateTime<Utc>>,
    wait: f64,
}

/// Requêtes laissées de côté dans chaque fenêtre : requêtes en vol, autres usages du même token.
pub fn reserve(quota: i32) -> i32 {
    (quota / 100).max(1)
}

/// Écart à imposer après une requête.
///
/// Tant qu'il reste plus d'un cinquième du quota, seul `min_interval` espace les
/// requêtes : un scan isolé reste rapide. En deçà, le reste est réparti jusqu'à
/// la remise à zéro ; à la réserve, on attend la remise à zéro.
pub fn pace(min_interval: f64, window: Option<Window>) -> Pace {
    let Some(window) = window.filter(|window| window.until_reset > 0.0) else {
        return Pace::Go(min_interval);
    };
    let reserve = reserve(window.quota);
    if window.remaining <= reserve {
        // GitHub announces whole seconds: one more avoids arriving just before the reset.
        return Pace::Wait(window.until_reset + 1.0);
    }
    if i64::from(window.remaining) * 5 > i64::from(window.quota) {
        return Pace::Go(min_interval);
    }
    Pace::Go((window.until_reset / f64::from(window.remaining - reserve)).max(min_interval))
}

/// Réserve une requête dans le budget du scan et sur une ressource de quota partagée.
///
/// `min_interval` espace les requêtes de tous les workers ; l'état annoncé par
/// GitHub peut imposer davantage (voir [`pace`]).
pub async fn reserve_request(
    pool: &PgPool,
    scan: &Scan,
    resource: &str,
    min_interval: Duration,
) -> Result<(), CollectError> {
    let mut tx = pool.begin().await?;
    let row = guard(&mut tx, scan).await?;
    if row.requests_used >= row.max_requests {
        return Err(CollectError::BudgetExceeded);
    }
    let (delay, remaining, quota, until_reset): (f64, Option<i32>, Option<i32>, Option<f64>) =
        sqlx::query_as(
            "SELECT extract(epoch FROM next_request_at - now())::float8, remaining, quota, \
         extract(epoch FROM reset_at - now())::float8 \
         FROM github_rate_limits WHERE resource = $1 FOR UPDATE",
        )
        .bind(resource)
        .fetch_one(&mut *tx)
        .await?;
    if delay > 0.0 {
        return Err(CollectError::RateLimited(delay));
    }
    let window = match (remaining, quota, until_reset) {
        (Some(remaining), Some(quota), Some(until_reset)) => Some(Window { remaining, quota, until_reset }),
        _ => None,
    };
    match pace(min_interval.as_secs_f64(), window) {
        Pace::Wait(seconds) => {
            let seconds = seconds.min(86_400.0);
            sqlx::query(
                "UPDATE github_rate_limits SET next_request_at = now() + make_interval(secs => $2) \
                 WHERE resource = $1",
            )
            .bind(resource)
            .bind(seconds)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Err(CollectError::RateLimited(seconds))
        }
        Pace::Go(spacing) => {
            sqlx::query(
                "UPDATE github_rate_limits SET next_request_at = now() + make_interval(secs => $2), \
                 remaining = CASE WHEN reset_at > now() AND remaining > 0 THEN remaining - 1 ELSE remaining END \
                 WHERE resource = $1",
            )
            .bind(resource)
            .bind(spacing.min(86_400.0))
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE scans SET requests_used = requests_used + 1 WHERE id = $1")
                .bind(scan.id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(())
        }
    }
}

/// Reporte l'état annoncé par une réponse GitHub.
///
/// Les réponses d'une même fenêtre peuvent arriver dans le désordre : on garde
/// la plus basse, sans revenir à une fenêtre plus ancienne. `refund` rend la
/// requête réservée qu'une réponse 304 authentifiée n'a pas décomptée.
pub async fn record_rate(pool: &PgPool, rate: &RateLimit, refund: bool) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO github_rate_limits AS r(resource, remaining, quota, reset_at) \
         VALUES($1, greatest($2, 0), greatest($3, 1), to_timestamp($4::float8)) \
         ON CONFLICT(resource) DO UPDATE SET \
         remaining = CASE \
           WHEN r.reset_at IS NULL OR r.reset_at < excluded.reset_at THEN excluded.remaining \
           WHEN r.reset_at = excluded.reset_at \
             THEN least(coalesce(r.remaining, excluded.remaining) + $5, excluded.remaining) \
           ELSE r.remaining END, \
         quota = CASE WHEN r.reset_at IS NULL OR r.reset_at <= excluded.reset_at THEN excluded.quota ELSE r.quota END, \
         reset_at = greatest(r.reset_at, excluded.reset_at), updated_at = now()",
    )
    .bind(&rate.resource)
    .bind(rate.remaining)
    .bind(rate.limit)
    .bind(epoch(rate.reset))
    .bind(i32::from(refund))
    .execute(pool)
    .await
    .map(|_| ())
}

/// Remplace l'estimation par un relevé de `GET /rate_limit`, qui fait foi à son instant.
pub async fn sync_rate(pool: &PgPool, rate: &RateLimit) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO github_rate_limits(resource, remaining, quota, reset_at) \
         VALUES($1, greatest($2, 0), greatest($3, 1), to_timestamp($4::float8)) \
         ON CONFLICT(resource) DO UPDATE SET remaining = excluded.remaining, quota = excluded.quota, \
         reset_at = excluded.reset_at, updated_at = now()",
    )
    .bind(&rate.resource)
    .bind(rate.remaining)
    .bind(rate.limit)
    .bind(epoch(rate.reset))
    .execute(pool)
    .await
    .map(|_| ())
}

/// Suspend les requêtes sur une ressource pendant `seconds` (1 s à 24 h).
pub async fn pause(pool: &PgPool, resource: &str, seconds: f64) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO github_rate_limits(resource, next_request_at) VALUES($1, now() + make_interval(secs => $2)) \
         ON CONFLICT(resource) DO UPDATE SET \
         next_request_at = greatest(github_rate_limits.next_request_at, excluded.next_request_at)",
    )
    .bind(resource)
    .bind(seconds.clamp(1.0, 86_400.0))
    .execute(pool)
    .await
    .map(|_| ())
}

/// Rend au scan une requête réservée que GitHub n'a pas décomptée (réponse 304 authentifiée).
pub async fn refund_request(pool: &PgPool, scan: &Scan) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE scans SET requests_used = greatest(requests_used - 1, 0) \
         WHERE id = $1 AND status = 'running' AND generation = $2",
    )
    .bind(scan.id)
    .bind(scan.generation)
    .execute(pool)
    .await
    .map(|_| ())
}

/// État des quotas pour `observatory github-status` : relevé de GitHub (non
/// décompté), reporté dans la base, puis rythme partagé et cache.
pub async fn status(pool: &PgPool, github: &GitHub) -> anyhow::Result<String> {
    let mut report = String::new();
    let access = if github.authenticated() { "token" } else { "anonyme (60 requêtes REST par heure)" };
    writeln!(report, "Accès GitHub : {access}")?;
    match github.rate_limits().await {
        Ok(limits) => {
            for rate in &limits {
                sync_rate(pool, rate).await?;
            }
        }
        Err(error) => writeln!(report, "Relevé GitHub indisponible : {error}")?,
    }
    let limits: Vec<Limit> = sqlx::query_as(
        "SELECT resource, remaining, quota, reset_at, \
         greatest(extract(epoch FROM next_request_at - now()), 0)::float8 AS wait \
         FROM github_rate_limits ORDER BY resource",
    )
    .fetch_all(pool)
    .await?;
    for Limit { resource, remaining, quota, reset_at, wait } in limits {
        let state = match (remaining, quota, reset_at) {
            (Some(remaining), Some(quota), Some(reset_at)) if reset_at > Utc::now() => format!(
                "{remaining}/{quota} restantes, remise à zéro {}",
                reset_at.format("%Y-%m-%d %H:%M:%S UTC")
            ),
            _ => "pas de fenêtre en cours".to_owned(),
        };
        writeln!(report, "{resource} : {state} ; prochaine requête dans {wait:.0} s")?;
    }
    let (entries, mine): (i64, i64) =
        sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE identity = $1) FROM http_cache")
            .bind(github.identity())
            .fetch_one(pool)
            .await?;
    write!(report, "Cache HTTP : {entries} réponses, dont {mine} pour cet accès")?;
    Ok(report)
}

fn epoch(seconds: i64) -> f64 {
    // Epoch seconds are exact in an f64 for the next 285 million years.
    #[allow(clippy::cast_precision_loss)]
    let value = seconds as f64;
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(remaining: i32, quota: i32, until_reset: f64) -> Window {
        Window { remaining, quota, until_reset }
    }

    #[test]
    fn unknown_or_past_windows_only_apply_the_minimum_interval() {
        assert_eq!(pace(0.25, None), Pace::Go(0.25));
        assert_eq!(pace(0.25, Some(window(0, 60, -3.0))), Pace::Go(0.25));
    }

    #[test]
    fn a_comfortable_quota_is_used_at_full_speed() {
        assert_eq!(pace(0.25, Some(window(4000, 5000, 1800.0))), Pace::Go(0.25));
        assert_eq!(pace(0.0, Some(window(13, 60, 3000.0))), Pace::Go(0.0));
    }

    #[test]
    fn the_last_fifth_is_spread_until_the_reset() {
        // 1000 left out of 5000, 50 kept in reserve: 950 requests over 1900 s.
        assert_eq!(pace(0.25, Some(window(1000, 5000, 1900.0))), Pace::Go(2.0));
        // Never faster than the minimum interval.
        assert_eq!(pace(5.0, Some(window(1000, 5000, 1900.0))), Pace::Go(5.0));
        // Anonymous access: 12 left out of 60, one in reserve.
        assert_eq!(pace(0.25, Some(window(12, 60, 1100.0))), Pace::Go(100.0));
    }

    #[test]
    fn the_reserve_waits_for_the_reset() {
        assert_eq!(reserve(60), 1);
        assert_eq!(reserve(5000), 50);
        assert_eq!(pace(0.25, Some(window(50, 5000, 600.0))), Pace::Wait(601.0));
        assert_eq!(pace(0.25, Some(window(0, 60, 10.0))), Pace::Wait(11.0));
    }
}
