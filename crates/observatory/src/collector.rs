//! Collecte bornée d'un voisinage GitHub public.
//!
//! `GitHub` ne parle qu'HTTP : limites de taille et de durée, quotas, origine.
//! `collect` ajoute la base : budget partagé, lease du scan, couverture par
//! endpoint. « Complet » signifie qu'un endpoint n'avait plus de page, jamais
//! que toutes les relations sont connues.

use std::time::Duration;

use chrono::Utc;
use futures_util::StreamExt;
use observatory_domain::pagination::{ForbiddenLink, api_path, next_page};
use observatory_domain::{GITHUB_API, SeedKind, parse_seed};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::{Map, Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::model::Scan;
use crate::store::{Role, ingest_github};

/// Taille maximale d'une réponse GitHub.
pub const MAX_BODY: usize = 2 * 1024 * 1024;
/// Durée maximale d'une requête GitHub, corps compris.
pub const MAX_DURATION: Duration = Duration::from_secs(30);
/// Pages lues au plus par endpoint dans un scan.
pub const MAX_PAGES: usize = 2;
/// Attente du budget partagé faite sur place plutôt qu'en remettant le scan en file.
const SHORT_WAIT: f64 = 5.0;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FetchError {
    #[error("Origine API non autorisée")]
    Origin,
    #[error("Réponse trop volumineuse ou trop lente")]
    TooLarge,
    #[error("HTTP {0} : ressource non disponible (aucune suppression inférée)")]
    Status(u16),
    #[error("Réponse JSON invalide")]
    Json,
    #[error("Échec réseau ; collecte incomplète")]
    Network,
    #[error("Quota GitHub atteint")]
    RateLimited(f64),
}

/// Réponse GitHub et éventuelle pause à imposer au budget partagé.
#[derive(Debug)]
pub struct Fetched {
    pub result: Result<(Value, String), FetchError>,
    /// Secondes pendant lesquelles plus aucun scan ne doit interroger GitHub.
    pub pause: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct GitHub {
    client: reqwest::Client,
    base: String,
}

impl GitHub {
    pub fn new(token: &str) -> anyhow::Result<Self> {
        Self::with_base(GITHUB_API, token)
    }

    /// Client vers une autre base d'API : réservé aux tests, qui simulent GitHub.
    pub fn with_base(base: &str, token: &str) -> anyhow::Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.github+json"));
        headers.insert("x-github-api-version", HeaderValue::from_static("2022-11-28"));
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("github-investigation/", env!("CARGO_PKG_VERSION"))),
        );
        if !token.is_empty() {
            let mut value = HeaderValue::from_str(&format!("Bearer {token}"))?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(10))
            .timeout(MAX_DURATION)
            .build()?;
        Ok(Self { client, base: base.trim_end_matches('/').to_owned() })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub async fn fetch(&self, url: &str) -> Fetched {
        if api_path(url, &self.base).is_none() {
            return Fetched { result: Err(FetchError::Origin), pause: None };
        }
        let response = match self.client.get(url).send().await {
            Ok(response) => response,
            Err(error) => return Fetched { result: Err(transport(&error)), pause: None },
        };
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) if body.len() + chunk.len() <= MAX_BODY => body.extend_from_slice(&chunk),
                Ok(_) => return Fetched { result: Err(FetchError::TooLarge), pause: None },
                Err(error) => return Fetched { result: Err(transport(&error)), pause: None },
            }
        }
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let mut pause = None;
        if header("x-ratelimit-remaining") == Some("0") {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |elapsed| elapsed.as_secs_f64());
            let delay = header("x-ratelimit-reset")
                .and_then(|reset| reset.parse::<f64>().ok())
                .map_or(60.0, |reset| (reset - now).max(60.0));
            pause = Some(delay);
            if matches!(status, 403 | 429) {
                return Fetched { result: Err(FetchError::RateLimited(delay)), pause };
            }
        }
        if status == 429 || (status == 403 && headers.contains_key("retry-after")) {
            let delay = header("retry-after").unwrap_or("60").parse::<f64>().unwrap_or(60.0);
            return Fetched { result: Err(FetchError::RateLimited(delay)), pause: Some(delay) };
        }
        if status != 200 {
            return Fetched { result: Err(FetchError::Status(status)), pause };
        }
        let result = serde_json::from_slice(&body)
            .map(|payload| (payload, header("link").unwrap_or_default().to_owned()))
            .map_err(|_| FetchError::Json);
        Fetched { result, pause }
    }
}

fn transport(error: &reqwest::Error) -> FetchError {
    if error.is_timeout() { FetchError::TooLarge } else { FetchError::Network }
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

/// Réserve une requête dans le budget du scan et dans le budget GitHub partagé.
///
/// `interval` espace les requêtes de tous les workers.
pub async fn reserve_request(pool: &PgPool, scan: &Scan, interval: Duration) -> Result<(), CollectError> {
    let mut tx = pool.begin().await?;
    let row = guard(&mut tx, scan).await?;
    if row.requests_used >= row.max_requests {
        return Err(CollectError::BudgetExceeded);
    }
    let delay: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM next_request_at - now())::float8 FROM github_budget WHERE id = 1 FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await?;
    if delay > 0.0 {
        return Err(CollectError::RateLimited(delay));
    }
    sqlx::query("UPDATE github_budget SET next_request_at = now() + make_interval(secs => $1) WHERE id = 1")
        .bind(interval.as_secs_f64())
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE scans SET requests_used = requests_used + 1 WHERE id = $1")
        .bind(scan.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Suspend toutes les requêtes GitHub pendant `seconds` (1 s à 24 h).
pub async fn pause_budget(pool: &PgPool, seconds: f64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE github_budget SET next_request_at = greatest(next_request_at, now() + make_interval(secs => $1)) \
         WHERE id = 1",
    )
    .bind(seconds.clamp(1.0, 86_400.0))
    .execute(pool)
    .await
    .map(|_| ())
}

/// Un scan en cours d'exécution, avec ses dépendances.
#[derive(Debug)]
pub struct Collector<'a> {
    pub pool: &'a PgPool,
    pub github: &'a GitHub,
    pub scan: &'a Scan,
    pub interval: Duration,
}

impl Collector<'_> {
    async fn get(&self, url: &str) -> Result<(Value, String), CollectError> {
        loop {
            match reserve_request(self.pool, self.scan, self.interval).await {
                Err(CollectError::RateLimited(delay)) if delay <= SHORT_WAIT => {
                    tokio::time::sleep(Duration::from_secs_f64(delay)).await;
                }
                other => break other?,
            }
        }
        let fetched = self.github.fetch(url).await;
        if let Some(seconds) = fetched.pause {
            pause_budget(self.pool, seconds).await?;
        }
        fetched.result.map_err(|error| match error {
            FetchError::RateLimited(delay) => CollectError::RateLimited(delay),
            other => CollectError::Fetch(other.to_string()),
        })
    }

    async fn save_coverage(&self, coverage: &Map<String, Value>) -> Result<(), CollectError> {
        let mut tx = self.pool.begin().await?;
        guard(&mut tx, self.scan).await?;
        sqlx::query("UPDATE scans SET coverage = $1 WHERE id = $2")
            .bind(Value::Object(coverage.clone()))
            .bind(self.scan.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Lit jusqu'à `MAX_PAGES` pages d'un endpoint et enregistre chaque page dans une transaction.
    async fn endpoint(
        &self,
        role: Role,
        mut url: String,
        root: Uuid,
        canonical: Option<&str>,
        result: &mut Map<String, Value>,
    ) -> Result<(), CollectError> {
        for _ in 0..MAX_PAGES {
            let (payload, link) = self.get(&url).await?;
            let Value::Array(items) = payload else {
                return Err(CollectError::Fetch("Liste attendue".into()));
            };
            let observed = Utc::now();
            let mut tx = self.pool.begin().await?;
            guard(&mut tx, self.scan).await?;
            for item in &items {
                if !item.is_object() {
                    return Err(CollectError::Fetch("Objet GitHub invalide".into()));
                }
                ingest_github(&mut tx, self.scan.id, role, item, &url, observed, Some(root)).await?;
            }
            tx.commit().await?;
            increment(result, "items", items.len());
            increment(result, "pages", 1);
            let Some(following) = next_page(&link, &url, canonical, self.github.base())? else {
                let status = if role == Role::Events { "windowed" } else { "complete" };
                result.insert("status".into(), status.into());
                return Ok(());
            };
            url = following;
        }
        Ok(())
    }

    /// Profil borné : la seed, puis ses contributeurs, forks, issues et événements
    /// (dépôt) ou ses dépôts (compte). Retourne `succeeded` ou `partial`.
    pub async fn collect(&self) -> Result<&'static str, CollectError> {
        let seed = parse_seed(&self.scan.seed).map_err(|error| CollectError::Fetch(error.to_string()))?;
        let base = self.github.base();
        let mut coverage = self.scan.coverage.as_object().cloned().unwrap_or_default();
        let repository = seed.kind == SeedKind::Repository;
        let root_url = format!("{base}{}{}", if repository { "/repos/" } else { "/users/" }, seed.path);
        let (root, _) = self.get(&root_url).await?;
        if !root.is_object() {
            return Err(CollectError::Fetch("Métadonnées invalides".into()));
        }
        if crate::store::is_private(&root) {
            return Err(CollectError::Fetch("Les dépôts privés sont hors périmètre".into()));
        }
        let mut tx = self.pool.begin().await?;
        guard(&mut tx, self.scan).await?;
        let role = if repository { Role::Repository } else { Role::Account };
        let root_id = ingest_github(&mut tx, self.scan.id, role, &root, &root_url, Utc::now(), None).await?;
        tx.commit().await?;
        let Some(root_id) = root_id else {
            return Err(CollectError::Fetch("Identité GitHub manquante".into()));
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
                (Role::Contributors, format!("{root_url}/contributors?per_page=50")),
                (Role::Forks, format!("{root_url}/forks?per_page=50")),
                (Role::Issues, format!("{root_url}/issues?state=all&per_page=50")),
                (Role::Events, format!("{root_url}/events?per_page=50")),
            ]
        } else {
            let owner = if root.get("type").and_then(Value::as_str) == Some("Organization") {
                "orgs"
            } else {
                "users"
            };
            vec![(Role::Repositories, format!("{base}/{owner}/{}/repos?type=owner&per_page=50", seed.path))]
        };
        // GitHub paginates /repos/<owner>/<name> through /repositories/<stable id>.
        let stable_id = root.get("id").filter(|id| id.is_u64()).map(ToString::to_string);
        for (role, url) in plan {
            let canonical = stable_id
                .as_ref()
                .filter(|_| repository)
                .map(|id| format!("/repositories/{id}/{}", role.as_str()));
            let mut result = Map::new();
            result.insert("status".into(), "partial".into());
            result.insert("items".into(), 0.into());
            result.insert("pages".into(), 0.into());
            let outcome = self.endpoint(role, url, root_id, canonical.as_deref(), &mut result).await;
            if result["status"] == "partial" {
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
                // Lease loss, quota waits and database errors end the scan; coverage is kept first.
                Err(other) => Some(other),
            };
            coverage.insert(role.as_str().into(), Value::Object(result));
            self.save_coverage(&coverage).await?;
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

fn increment(result: &mut Map<String, Value>, key: &str, by: usize) {
    let current = result.get(key).and_then(Value::as_u64).unwrap_or(0);
    result.insert(key.into(), (current + by as u64).into());
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn github(server: &MockServer) -> GitHub {
        GitHub::with_base(&server.uri(), "test-token").unwrap()
    }

    #[tokio::test]
    async fn returns_payload_and_link() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/a/b"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id": 1}))
                    .insert_header("link", "<x>; rel=\"next\""),
            )
            .mount(&server)
            .await;
        let fetched = github(&server).fetch(&format!("{}/repos/a/b", server.uri())).await;
        assert_eq!(fetched.result, Ok((json!({"id": 1}), "<x>; rel=\"next\"".into())));
        assert_eq!(fetched.pause, None);
        let request = &server.received_requests().await.unwrap()[0];
        assert_eq!(request.headers["authorization"], "Bearer test-token");
        assert_eq!(request.headers["x-github-api-version"], "2022-11-28");
    }

    #[tokio::test]
    async fn refuses_other_origins_without_a_request() {
        let server = MockServer::start().await;
        let fetched = github(&server).fetch("https://api.github.com.evil.test/x").await;
        assert_eq!(fetched.result, Err(FetchError::Origin));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn caps_body_size() {
        let server = MockServer::start().await;
        let body = format!("[\"{}\"]", "x".repeat(MAX_BODY));
        Mock::given(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        let fetched = github(&server).fetch(&format!("{}/big", server.uri())).await;
        assert_eq!(fetched.result, Err(FetchError::TooLarge));
    }

    #[tokio::test]
    async fn retry_after_pauses_the_shared_budget() {
        let server = MockServer::start().await;
        Mock::given(path("/limited"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
            .mount(&server)
            .await;
        let fetched = github(&server).fetch(&format!("{}/limited", server.uri())).await;
        assert_eq!(fetched.result, Err(FetchError::RateLimited(120.0)));
        assert_eq!(fetched.pause, Some(120.0));
    }

    #[tokio::test]
    async fn exhausted_quota_waits_at_least_a_minute() {
        let server = MockServer::start().await;
        let soon = (Utc::now().timestamp() + 5).to_string();
        Mock::given(path("/forbidden"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("x-ratelimit-reset", soon.as_str()),
            )
            .mount(&server)
            .await;
        Mock::given(path("/last"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([]))
                    .insert_header("x-ratelimit-remaining", "0"),
            )
            .mount(&server)
            .await;
        let client = github(&server);
        let forbidden = client.fetch(&format!("{}/forbidden", server.uri())).await;
        assert_eq!(forbidden.result, Err(FetchError::RateLimited(60.0)));
        // The last allowed answer is kept, and the budget still pauses.
        let last = client.fetch(&format!("{}/last", server.uri())).await;
        assert_eq!(last.result, Ok((json!([]), String::new())));
        assert_eq!(last.pause, Some(60.0));
    }

    #[tokio::test]
    async fn errors_never_infer_deletion_and_redirects_are_not_followed() {
        let server = MockServer::start().await;
        Mock::given(path("/gone")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        Mock::given(path("/moved"))
            .respond_with(ResponseTemplate::new(301).insert_header("location", "https://evil.test/"))
            .mount(&server)
            .await;
        Mock::given(path("/html"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
            .mount(&server)
            .await;
        let client = github(&server);
        let gone = client.fetch(&format!("{}/gone", server.uri())).await.result.unwrap_err();
        assert_eq!(gone.to_string(), "HTTP 404 : ressource non disponible (aucune suppression inférée)");
        assert_eq!(
            client.fetch(&format!("{}/moved", server.uri())).await.result,
            Err(FetchError::Status(301))
        );
        assert_eq!(client.fetch(&format!("{}/html", server.uri())).await.result, Err(FetchError::Json));
    }
}
