//! Collecte bornée d'un voisinage GitHub public.
//!
//! [`http`] ne parle qu'HTTP : limites de taille et de durée, origine, en-têtes
//! de quota. [`budget`] partage le rythme des requêtes entre workers, [`cache`]
//! garde les réponses réutilisables. [`Collector`] ajoute le lease du scan, les
//! points de reprise et la couverture par endpoint. « Complet » signifie qu'un
//! endpoint n'avait plus de page, jamais que toutes les relations sont connues.

pub mod budget;
pub mod cache;
pub mod http;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use observatory_domain::pagination::{ForbiddenLink, next_page};
use observatory_domain::{SeedKind, parse_seed};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

pub use self::budget::{CORE, reserve_request};
pub use self::http::{FetchError, GitHub, Response};
use crate::model::Scan;
use crate::store::{Batch, EntityRef, Role, ingest_github};

/// Pages lues au plus par endpoint dans un scan, reprises comprises.
pub const MAX_PAGES: usize = 2;
/// Éléments demandés par page, le maximum de l'API REST.
pub const PER_PAGE: u32 = 100;
/// Attente du budget partagé faite sur place plutôt qu'en remettant le scan en file.
const SHORT_WAIT: f64 = 5.0;
/// Nouvelles tentatives après une erreur passagère (réseau, 5xx), chacune réservée dans le budget.
const MAX_RETRIES: u32 = 2;

/// Réglages de collecte du worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Écart minimal entre deux requêtes GitHub, tous workers confondus.
    pub min_interval: Duration,
    /// Durée pendant laquelle une réponse en cache est réutilisée sans interroger GitHub.
    pub freshness: Duration,
    /// Conservation des réponses en cache que GitHub n'a plus confirmées.
    pub retention: Duration,
    /// Attente avant la première nouvelle tentative ; triplée ensuite, avec une part aléatoire.
    pub backoff: Duration,
    /// Scans exécutés en même temps par un worker.
    pub concurrency: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            min_interval: Duration::from_millis(250),
            freshness: Duration::from_secs(600),
            retention: Duration::from_hours(7 * 24),
            backoff: Duration::from_secs(1),
            concurrency: 2,
        }
    }
}

impl Settings {
    /// Sans espacement, ni réutilisation directe, ni attente entre tentatives : pour les tests.
    pub const IMMEDIATE: Self = Self {
        min_interval: Duration::ZERO,
        freshness: Duration::ZERO,
        retention: Duration::from_hours(7 * 24),
        backoff: Duration::ZERO,
        concurrency: 1,
    };

    /// `GITHUB_MIN_INTERVAL_MS`, `GITHUB_CACHE_FRESHNESS_SECONDS`, `GITHUB_CACHE_RETENTION_DAYS`
    /// et `WORKER_CONCURRENCY`.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::parse(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
    }

    pub fn parse(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let defaults = Self::default();
        let number = |name: &str, max: u64| -> anyhow::Result<Option<u64>> {
            let Some(value) = get(name) else { return Ok(None) };
            let value: u64 = value.trim().parse().with_context(|| format!("{name} : entier attendu"))?;
            anyhow::ensure!(value <= max, "{name} : {max} au plus");
            Ok(Some(value))
        };
        Ok(Self {
            min_interval: number("GITHUB_MIN_INTERVAL_MS", 60_000)?
                .map_or(defaults.min_interval, Duration::from_millis),
            freshness: number("GITHUB_CACHE_FRESHNESS_SECONDS", 86_400)?
                .map_or(defaults.freshness, Duration::from_secs),
            retention: number("GITHUB_CACHE_RETENTION_DAYS", 365)?
                .map_or(defaults.retention, |days| Duration::from_hours(days * 24)),
            backoff: defaults.backoff,
            concurrency: match number("WORKER_CONCURRENCY", 8)? {
                Some(0) => anyhow::bail!("WORKER_CONCURRENCY : 1 au moins"),
                Some(lanes) => usize::try_from(lanes)?,
                None => defaults.concurrency,
            },
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CollectError {
    /// Le scan a été annulé ou repris par un autre worker.
    #[error("lease lost")]
    LeaseLost,
    #[error("Budget de requêtes atteint")]
    BudgetExceeded,
    #[error("Attente du quota GitHub partagé")]
    RateLimited(f64),
    #[error("{0}")]
    Fetch(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl From<ForbiddenLink> for CollectError {
    fn from(error: ForbiddenLink) -> Self {
        Self::Fetch(error.to_string())
    }
}

/// Verrouille le scan, vérifie qu'il appartient encore à ce worker et prolonge son lease.
pub async fn guard(conn: &mut PgConnection, scan: &Scan) -> Result<Scan, CollectError> {
    let row: Option<Scan> = sqlx::query_as("SELECT * FROM scans WHERE id = $1 FOR UPDATE")
        .bind(scan.id)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(row) = row.filter(|row| row.status == "running" && row.generation == scan.generation) else {
        return Err(CollectError::LeaseLost);
    };
    sqlx::query("UPDATE scans SET lease_until = now() + interval '90 seconds' WHERE id = $1")
        .bind(scan.id)
        .execute(conn)
        .await?;
    Ok(row)
}

/// Compteurs d'un passage, ajoutés à `coverage.stats`.
#[derive(Debug, Default)]
struct Stats {
    requests: AtomicU64,
    not_modified: AtomicU64,
    cache_hits: AtomicU64,
    retries: AtomicU64,
}

impl Stats {
    fn add(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Cumul avec les passages précédents du même scan.
    fn merged(&self, previous: Option<&Value>) -> Value {
        let total = |key: &str, counter: &AtomicU64| {
            previous.and_then(|stats| stats.get(key)).and_then(Value::as_u64).unwrap_or(0)
                + counter.load(Ordering::Relaxed)
        };
        json!({
            "requests": total("requests", &self.requests),
            "not_modified": total("not_modified", &self.not_modified),
            "cache_hits": total("cache_hits", &self.cache_hits),
            "retries": total("retries", &self.retries),
        })
    }
}

/// Page d'une réponse GitHub, et l'instant auquel GitHub l'a confirmée.
struct Page {
    payload: Value,
    link: String,
    observed: DateTime<Utc>,
}

/// Point de reprise de la seed : ce que les endpoints suivants en utilisent.
#[derive(Debug, Serialize, Deserialize)]
struct Root {
    entity: Uuid,
    /// Identifiant stable GitHub, qui sert aux liens de pagination canoniques.
    id: Option<u64>,
    organization: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct Checkpoint {
    step: String,
    next_url: Option<String>,
    state: Value,
    done: bool,
}

async fn save_checkpoint(
    conn: &mut PgConnection,
    scan: Uuid,
    step: &str,
    next_url: Option<&str>,
    state: &Value,
    done: bool,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO scan_checkpoints(scan_id, step, next_url, state, done) VALUES($1, $2, $3, $4, $5) \
         ON CONFLICT(scan_id, step) DO UPDATE SET next_url = excluded.next_url, state = excluded.state, \
         done = excluded.done, updated_at = now()",
    )
    .bind(scan)
    .bind(step)
    .bind(next_url)
    .bind(state)
    .bind(done)
    .execute(conn)
    .await
    .map(|_| ())
}

/// Un scan en cours d'exécution, avec ses dépendances.
#[derive(Debug)]
pub struct Collector<'a> {
    pool: &'a PgPool,
    github: &'a GitHub,
    scan: &'a Scan,
    settings: Settings,
    stats: Stats,
}

impl<'a> Collector<'a> {
    pub fn new(pool: &'a PgPool, github: &'a GitHub, scan: &'a Scan, settings: Settings) -> Self {
        Self { pool, github, scan, settings, stats: Stats::default() }
    }

    /// Attend sur place une réservation proche ; une attente longue remet le scan en file.
    async fn reserve(&self) -> Result<(), CollectError> {
        loop {
            match reserve_request(self.pool, self.scan, CORE, self.settings.min_interval).await {
                Err(CollectError::RateLimited(delay)) if delay <= SHORT_WAIT => {
                    tokio::time::sleep(Duration::from_secs_f64(delay)).await;
                }
                other => return other,
            }
        }
    }

    /// GET d'une URL : cache frais, sinon requête conditionnelle, avec nouvelles tentatives bornées.
    async fn get(&self, url: &str) -> Result<Page, CollectError> {
        let identity = self.github.identity();
        let cached = cache::lookup(self.pool, url, identity, self.settings.freshness).await?;
        if let Some(hit) = cached.as_ref().filter(|hit| hit.fresh) {
            Stats::add(&self.stats.cache_hits);
            return Ok(Page {
                payload: hit.payload.clone(),
                link: hit.link.clone(),
                observed: hit.validated_at,
            });
        }
        let etag = cached.as_ref().and_then(|hit| hit.etag.as_deref());
        let mut attempt = 0;
        let fetched = loop {
            self.reserve().await?;
            let fetched = self.github.fetch(url, etag).await;
            Stats::add(&self.stats.requests);
            let refund = self.github.authenticated() && matches!(fetched.result, Ok(Response::NotModified));
            if let Some(rate) = &fetched.rate {
                budget::record_rate(self.pool, rate, refund).await?;
            }
            if refund {
                budget::refund_request(self.pool, self.scan).await?;
            }
            if let Some(seconds) = fetched.pause {
                let resource = fetched.rate.as_ref().map_or(CORE, |rate| rate.resource.as_str());
                budget::pause(self.pool, resource, seconds).await?;
            }
            match &fetched.result {
                Err(error) if error.is_transient() && attempt < MAX_RETRIES => {
                    attempt += 1;
                    Stats::add(&self.stats.retries);
                    tokio::time::sleep(backoff(self.settings.backoff, attempt)).await;
                }
                _ => break fetched,
            }
        };
        match fetched.result {
            Ok(Response::Fresh { payload, link, etag }) => {
                cache::store(self.pool, url, identity, etag.as_deref(), &payload, &link).await?;
                Ok(Page { payload, link, observed: Utc::now() })
            }
            Ok(Response::NotModified) => {
                let Some(hit) = cached else {
                    return Err(CollectError::Fetch("Réponse 304 sans version en cache".into()));
                };
                cache::revalidate(self.pool, url, identity).await?;
                Stats::add(&self.stats.not_modified);
                Ok(Page { payload: hit.payload, link: hit.link, observed: Utc::now() })
            }
            Err(FetchError::RateLimited(delay)) => Err(CollectError::RateLimited(delay)),
            Err(other) => Err(CollectError::Fetch(other.to_string())),
        }
    }

    async fn checkpoints(&self) -> Result<HashMap<String, Checkpoint>, CollectError> {
        let rows: Vec<Checkpoint> =
            sqlx::query_as("SELECT step, next_url, state, done FROM scan_checkpoints WHERE scan_id = $1")
                .bind(self.scan.id)
                .fetch_all(self.pool)
                .await?;
        Ok(rows.into_iter().map(|row| (row.step.clone(), row)).collect())
    }

    /// Enregistre la couverture et ses compteurs ; `finished` clôt une étape dans la même transaction.
    async fn save_coverage(
        &self,
        coverage: &mut Map<String, Value>,
        previous: Option<&Value>,
        finished: Option<&str>,
    ) -> Result<(), CollectError> {
        coverage.insert("stats".into(), self.stats.merged(previous));
        let mut tx = self.pool.begin().await?;
        guard(&mut tx, self.scan).await?;
        sqlx::query("UPDATE scans SET coverage = $1 WHERE id = $2")
            .bind(Value::Object(coverage.clone()))
            .bind(self.scan.id)
            .execute(&mut *tx)
            .await?;
        if let Some(step) = finished {
            save_checkpoint(&mut tx, self.scan.id, step, None, &coverage[step], true).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Lit la seed, l'enregistre et pose son point de reprise dans la même transaction.
    async fn root(&self, url: &str, repository: bool) -> Result<Root, CollectError> {
        let page = self.get(url).await?;
        let root = page.payload;
        if !root.is_object() {
            return Err(CollectError::Fetch("Métadonnées invalides".into()));
        }
        if crate::store::is_private(&root) {
            return Err(CollectError::Fetch("Les dépôts privés sont hors périmètre".into()));
        }
        let mut tx = self.pool.begin().await?;
        guard(&mut tx, self.scan).await?;
        let role = if repository { Role::Repository } else { Role::Account };
        let Some(entity) =
            ingest_github(&mut tx, self.scan.id, role, &root, url, page.observed, None).await?
        else {
            return Err(CollectError::Fetch("Identité GitHub manquante".into()));
        };
        let state = Root {
            entity,
            id: root.get("id").and_then(Value::as_u64),
            organization: root.get("type").and_then(Value::as_str) == Some("Organization"),
        };
        let value = serde_json::to_value(&state).map_err(|error| CollectError::Fetch(error.to_string()))?;
        save_checkpoint(&mut tx, self.scan.id, "seed", None, &value, true).await?;
        tx.commit().await?;
        Ok(state)
    }

    /// Lit les pages d'un endpoint jusqu'à `MAX_PAGES` au total, reprises comprises.
    ///
    /// Chaque page est enregistrée avec son point de reprise dans une seule
    /// transaction : `next` et `result` ne bougent qu'une fois la page acquise.
    async fn endpoint(
        &self,
        role: Role,
        next: &mut Option<String>,
        root: Uuid,
        canonical: Option<&str>,
        result: &mut Map<String, Value>,
    ) -> Result<(), CollectError> {
        while let Some(url) = next.clone() {
            if count(result, "pages") >= MAX_PAGES as u64 {
                return Ok(());
            }
            let page = self.get(&url).await?;
            let Value::Array(items) = page.payload else {
                return Err(CollectError::Fetch("Liste attendue".into()));
            };
            let following = next_page(&page.link, &url, canonical, self.github.base());
            let mut progress = result.clone();
            increment(&mut progress, "items", items.len());
            increment(&mut progress, "pages", 1);
            if matches!(following, Ok(None)) {
                let status = if role == Role::Events { "windowed" } else { "complete" };
                progress.insert("status".into(), status.into());
            }
            let following_url = following.as_ref().ok().cloned().flatten();
            if items.iter().any(|item| !item.is_object()) {
                return Err(CollectError::Fetch("Objet GitHub invalide".into()));
            }
            let root = EntityRef::Id(root);
            let mut batch = Batch::default();
            for item in &items {
                batch.ingest_github(self.scan.id, role, item, &url, page.observed, Some(&root));
            }
            let mut tx = self.pool.begin().await?;
            guard(&mut tx, self.scan).await?;
            batch.write(&mut tx).await?;
            let state = Value::Object(progress.clone());
            save_checkpoint(&mut tx, self.scan.id, role.as_str(), following_url.as_deref(), &state, false)
                .await?;
            tx.commit().await?;
            *result = progress;
            *next = following_url;
            following?;
        }
        Ok(())
    }

    /// Profil borné : la seed, puis ses contributeurs, forks, issues et événements
    /// (dépôt) ou ses dépôts (compte). Retourne `succeeded` ou `partial`.
    ///
    /// Un scan repris saute les étapes terminées et reprend la suivante à sa
    /// dernière page enregistrée.
    pub async fn collect(&self) -> Result<&'static str, CollectError> {
        let seed = parse_seed(&self.scan.seed).map_err(|error| CollectError::Fetch(error.to_string()))?;
        let base = self.github.base();
        let mut coverage = self.scan.coverage.as_object().cloned().unwrap_or_default();
        let previous = coverage.get("stats").cloned();
        let repository = seed.kind == SeedKind::Repository;
        let root_url = format!("{base}{}{}", if repository { "/repos/" } else { "/users/" }, seed.path);
        let mut checkpoints = self.checkpoints().await?;
        let resumed = checkpoints
            .remove("seed")
            .filter(|checkpoint| checkpoint.done)
            .and_then(|checkpoint| serde_json::from_value::<Root>(checkpoint.state).ok());
        let root = match resumed {
            Some(root) => root,
            None => self.root(&root_url, repository).await?,
        };
        coverage.insert("seed".into(), json!({"status": "complete", "items": 1}));
        let plan = if repository {
            coverage.insert(
                "stargazers".into(),
                json!({"status": "not_requested", "reason": "Listings restreints ; événements partiels seulement"}),
            );
            coverage.insert(
                "watchers".into(),
                json!({"status": "not_requested", "reason": "Connecteur à qualifier"}),
            );
            vec![
                (Role::Contributors, format!("{root_url}/contributors?per_page={PER_PAGE}")),
                (Role::Forks, format!("{root_url}/forks?per_page={PER_PAGE}")),
                (Role::Issues, format!("{root_url}/issues?state=all&per_page={PER_PAGE}")),
                (Role::Events, format!("{root_url}/events?per_page={PER_PAGE}")),
            ]
        } else {
            let owner = if root.organization { "orgs" } else { "users" };
            vec![(
                Role::Repositories,
                format!("{base}/{owner}/{}/repos?type=owner&per_page={PER_PAGE}", seed.path),
            )]
        };
        for (role, url) in plan {
            let name = role.as_str();
            let checkpoint = checkpoints.remove(name);
            if checkpoint.as_ref().is_some_and(|checkpoint| checkpoint.done) {
                continue;
            }
            let mut result = Map::new();
            result.insert("status".into(), "partial".into());
            result.insert("items".into(), 0.into());
            result.insert("pages".into(), 0.into());
            let mut next = Some(url);
            if let Some(checkpoint) = checkpoint {
                result.extend(checkpoint.state.as_object().cloned().unwrap_or_default());
                next = checkpoint.next_url;
            }
            // GitHub paginates /repos/<owner>/<name> through /repositories/<stable id>.
            let canonical = root.id.filter(|_| repository).map(|id| format!("/repositories/{id}/{name}"));
            let outcome =
                self.endpoint(role, &mut next, root.entity, canonical.as_deref(), &mut result).await;
            if outcome.is_ok() && result["status"] == "partial" {
                result.insert("reason".into(), "Plafond de deux pages par endpoint atteint".into());
            }
            if role == Role::Events {
                result.insert(
                    "reason".into(),
                    "Flux partiel : au plus 300 événements / 30 jours, deux pages collectées au maximum"
                        .into(),
                );
            }
            let stop = match outcome {
                Ok(()) => None,
                Err(CollectError::BudgetExceeded) => {
                    result.insert("reason".into(), "Budget de requêtes atteint".into());
                    None
                }
                Err(CollectError::Fetch(reason)) => {
                    result.insert("status".into(), "error".into());
                    result.insert("reason".into(), reason.into());
                    None
                }
                // Lease loss, quota waits and database errors end this pass; the
                // checkpoint stays open so that the next pass resumes the endpoint.
                Err(other) => Some(other),
            };
            coverage.insert(name.into(), Value::Object(result));
            let finished = stop.is_none().then_some(name);
            self.save_coverage(&mut coverage, previous.as_ref(), finished).await?;
            if let Some(error) = stop {
                return Err(error);
            }
        }
        let partial = coverage
            .values()
            .any(|entry| matches!(entry.get("status").and_then(Value::as_str), Some("partial" | "error")));
        Ok(if partial { "partial" } else { "succeeded" })
    }
}

/// Attente avant la tentative `attempt` (à partir de 1) : `base`, 3 × `base`... à ±50 %.
fn backoff(base: Duration, attempt: u32) -> Duration {
    // Jitter spreads the retries of concurrent workers.
    base.mul_f64(f64::from(3_u32.pow(attempt.saturating_sub(1))) * (0.5 + rand::random::<f64>()))
}

fn count(result: &Map<String, Value>, key: &str) -> u64 {
    result.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn increment(result: &mut Map<String, Value>, key: &str, by: usize) {
    result.insert(key.into(), (count(result, key) + by as u64).into());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(pairs: &[(&str, &str)]) -> anyhow::Result<Settings> {
        let pairs: HashMap<String, String> =
            pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect();
        Settings::parse(|name| pairs.get(name).cloned())
    }

    #[test]
    fn settings_have_safe_defaults_and_bounds() {
        assert_eq!(parse(&[]).unwrap(), Settings::default());
        let tuned = parse(&[
            ("GITHUB_MIN_INTERVAL_MS", "1000"),
            ("GITHUB_CACHE_FRESHNESS_SECONDS", "0"),
            ("GITHUB_CACHE_RETENTION_DAYS", "30"),
        ])
        .unwrap();
        assert_eq!(
            (tuned.min_interval, tuned.freshness, tuned.retention),
            (Duration::from_secs(1), Duration::ZERO, Duration::from_hours(30 * 24))
        );
        assert!(parse(&[("GITHUB_MIN_INTERVAL_MS", "fast")]).is_err());
        assert!(parse(&[("GITHUB_CACHE_RETENTION_DAYS", "100000")]).is_err());
        assert_eq!(parse(&[("WORKER_CONCURRENCY", "4")]).unwrap().concurrency, 4);
        assert!(parse(&[("WORKER_CONCURRENCY", "0")]).is_err());
        assert!(parse(&[("WORKER_CONCURRENCY", "9")]).is_err());
    }

    #[test]
    fn statistics_accumulate_across_passes() {
        let stats = Stats::default();
        Stats::add(&stats.requests);
        Stats::add(&stats.cache_hits);
        let previous = json!({"requests": 4, "not_modified": 2});
        assert_eq!(
            stats.merged(Some(&previous)),
            json!({"requests": 5, "not_modified": 2, "cache_hits": 1, "retries": 0})
        );
    }

    #[test]
    fn backoff_grows_with_jitter() {
        assert_eq!(backoff(Duration::ZERO, 2), Duration::ZERO);
        for _ in 0..20 {
            let first = backoff(Duration::from_secs(1), 1);
            let second = backoff(Duration::from_secs(1), 2);
            assert!(first >= Duration::from_millis(500) && first < Duration::from_millis(1500));
            assert!(second >= Duration::from_millis(1500) && second < Duration::from_millis(4500));
        }
    }
}
