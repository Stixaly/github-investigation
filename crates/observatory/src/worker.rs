//! File de scans PostgreSQL : réservation `SKIP LOCKED`, lease et génération.
//!
//! La génération empêche un worker qui a perdu son lease (annulation, reprise
//! par un autre worker) d'écrire encore dans le scan.

use std::future::Future;
use std::time::Duration;

use futures_util::future::join_all;
use sqlx::PgPool;
use tokio::sync::watch;
use uuid::Uuid;

use crate::collector::{CollectError, Collector, GitHub, Settings, cache};
use crate::model::Scan;

/// Reprises tolérées pour un scan dont le worker a disparu.
pub const MAX_ATTEMPTS: i32 = 3;
/// Fréquence de purge des réponses GitHub en cache.
const PURGE_EVERY: Duration = Duration::from_secs(3600);

/// Réserve le plus ancien scan GitHub disponible, ou reprend un scan dont le lease a expiré.
pub async fn claim(pool: &PgPool) -> sqlx::Result<Option<Scan>> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE scans SET status = 'failed', error = 'Limite de reprises atteinte', finished_at = now() \
         WHERE status = 'running' AND lease_until < now() AND attempts >= $1",
    )
    .bind(MAX_ATTEMPTS)
    .execute(&mut *tx)
    .await?;
    let id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM scans WHERE source = 'github' AND available_at <= now() \
         AND (status IN ('queued', 'waiting_quota') OR (status = 'running' AND lease_until < now())) \
         ORDER BY requested_at FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(id) = id else {
        tx.commit().await?;
        return Ok(None);
    };
    let scan = sqlx::query_as(
        "UPDATE scans SET status = 'running', generation = generation + 1, attempts = attempts + 1, \
         started_at = coalesce(started_at, now()), lease_until = now() + interval '90 seconds', error = NULL \
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(scan))
}

/// Exécute un scan réservé et enregistre son état final si le lease est toujours détenu.
pub async fn execute(pool: &PgPool, github: &GitHub, scan: &Scan, settings: Settings) -> sqlx::Result<()> {
    let collector = Collector::new(pool, github, scan, settings);
    let (status, error, delay) = match collector.collect().await {
        Ok(status) => (status, None, 0.0),
        Err(CollectError::LeaseLost) => return Ok(()),
        Err(CollectError::RateLimited(seconds)) => {
            ("waiting_quota", Some("Attente du quota GitHub partagé".to_owned()), seconds.ceil() + 1.0)
        }
        Err(CollectError::BudgetExceeded) => {
            ("partial", Some("Budget de requêtes atteint pendant la reprise".to_owned()), 0.0)
        }
        Err(CollectError::Fetch(reason)) => ("failed", Some(reason), 0.0),
        Err(CollectError::Database(error)) => {
            // Provider payloads and credentials stay out of client-visible errors.
            tracing::error!(scan = %scan.id, %error, "scan failed");
            ("failed", Some("Erreur interne de collecte ; consulter le journal du worker".to_owned()), 0.0)
        }
    };
    sqlx::query(
        "UPDATE scans SET status = $1, error = $2, lease_until = NULL, \
         available_at = now() + make_interval(secs => $3::float8), \
         finished_at = CASE WHEN $1 = 'waiting_quota' THEN NULL ELSE now() END, \
         attempts = CASE WHEN $1 = 'waiting_quota' THEN greatest(attempts - 1, 0) ELSE attempts END \
         WHERE id = $4 AND status = 'running' AND generation = $5",
    )
    .bind(status)
    .bind(error)
    .bind(delay.min(86_400.0))
    .bind(scan.id)
    .bind(scan.generation)
    .execute(pool)
    .await?;
    tracing::info!(scan = %scan.id, status, "scan finished");
    Ok(())
}

/// Rend immédiatement un scan interrompu par l'arrêt du worker, sans attendre l'expiration du lease.
pub async fn release(pool: &PgPool, scan: &Scan) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE scans SET status = 'queued', lease_until = NULL, available_at = now(), \
         generation = generation + 1, attempts = greatest(attempts - 1, 0) \
         WHERE id = $1 AND status = 'running' AND generation = $2",
    )
    .bind(scan.id)
    .bind(scan.generation)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Boucle du worker, jusqu'au signal d'arrêt : `settings.concurrency` scans au plus
/// en même temps. Purge aussi le cache HTTP expiré, au démarrage puis toutes les heures.
///
/// Les voies d'exécution partagent la tâche, le pool et le rythme GitHub : la
/// concurrence recouvre les attentes réseau et l'écriture en base, sans dépasser
/// l'écart minimal entre deux requêtes.
pub async fn run(pool: PgPool, github: GitHub, settings: Settings, shutdown: impl Future<Output = ()>) {
    let (stop, stopped) = watch::channel(false);
    let lanes = join_all(
        (0..settings.concurrency.max(1))
            .map(|lane| run_lane(&pool, &github, settings, stopped.clone(), lane == 0)),
    );
    let signal = async {
        shutdown.await;
        // Every lane holds a receiver: the channel stays open until they return.
        let _ = stop.send(true);
    };
    tokio::join!(lanes, signal);
}

/// Une voie d'exécution : réserve un scan, l'exécute, recommence.
async fn run_lane(
    pool: &PgPool,
    github: &GitHub,
    settings: Settings,
    mut stopped: watch::Receiver<bool>,
    purges: bool,
) {
    let mut next_purge = tokio::time::Instant::now();
    loop {
        if *stopped.borrow() {
            return;
        }
        if purges && tokio::time::Instant::now() >= next_purge {
            next_purge = tokio::time::Instant::now() + PURGE_EVERY;
            match cache::purge(pool, settings.retention).await {
                Ok(0) => {}
                Ok(removed) => tracing::info!(removed, "expired HTTP cache entries purged"),
                Err(error) => tracing::error!(%error, "could not purge the HTTP cache"),
            }
        }
        let claimed = tokio::select! {
            _ = stopped.changed() => return,
            claimed = claim(pool) => claimed,
        };
        let pause = match claimed {
            Ok(Some(scan)) => {
                tracing::info!(scan = %scan.id, seed = %scan.seed, attempt = scan.attempts, "scan claimed");
                tokio::select! {
                    result = execute(pool, github, &scan, settings) => {
                        if let Err(error) = result {
                            tracing::error!(scan = %scan.id, %error, "could not record scan result");
                        }
                    }
                    _ = stopped.changed() => {
                        if let Err(error) = release(pool, &scan).await {
                            tracing::error!(scan = %scan.id, %error, "could not release scan");
                        }
                        return;
                    }
                }
                continue;
            }
            Ok(None) => Duration::from_secs(2),
            Err(error) => {
                tracing::error!(%error, "worker loop failed");
                Duration::from_secs(5)
            }
        };
        tokio::select! {
            _ = stopped.changed() => return,
            () = tokio::time::sleep(pause) => {}
        }
    }
}
