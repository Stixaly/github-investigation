//! Client HTTP de l'API GitHub : origine fixe, taille et durée bornées,
//! requêtes conditionnelles et lecture des en-têtes de quota.

use std::time::Duration;

use futures_util::StreamExt;
use observatory_domain::GITHUB_API;
use observatory_domain::pagination::api_path;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, IF_NONE_MATCH, USER_AGENT};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Taille maximale d'une réponse GitHub.
pub const MAX_BODY: usize = 2 * 1024 * 1024;
/// Durée maximale d'une requête GitHub, corps compris.
pub const MAX_DURATION: Duration = Duration::from_secs(30);
const MEDIA_TYPE: &str = "application/vnd.github+json";
const API_VERSION: &str = "2022-11-28";
/// Représentation demandée à GitHub : une réponse en cache ne sert que pour elle.
pub const REPRESENTATION: &str = "application/vnd.github+json; version=2022-11-28";

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

impl FetchError {
    /// Erreur passagère qui justifie une nouvelle tentative.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Network | Self::Status(500 | 502 | 503 | 504))
    }
}

/// État d'une ressource de quota GitHub (`core`, `search`, `graphql`), d'après les en-têtes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimit {
    pub resource: String,
    pub remaining: i32,
    pub limit: i32,
    /// Fin de la fenêtre courante, en secondes depuis l'époque Unix.
    pub reset: i64,
}

impl RateLimit {
    /// Lit l'objet d'une ressource dans la réponse de `GET /rate_limit`.
    pub fn from_resource(resource: &str, value: &Value) -> Option<Self> {
        let number = |key: &str| value.get(key).and_then(Value::as_i64);
        Some(Self {
            resource: valid_resource(resource)?.to_owned(),
            remaining: i32::try_from(number("remaining")?).ok()?,
            limit: i32::try_from(number("limit")?).ok()?,
            reset: number("reset")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// Corps JSON, en-tête `Link` et `ETag` éventuel.
    Fresh { payload: Value, link: String, etag: Option<String> },
    /// 304 : la version en cache est toujours la bonne.
    NotModified,
}

/// Résultat d'un appel, avec ce que le budget partagé doit en retenir.
#[derive(Debug)]
pub struct Fetched {
    pub result: Result<Response, FetchError>,
    /// Secondes pendant lesquelles plus aucun scan ne doit interroger cette ressource.
    pub pause: Option<f64>,
    pub rate: Option<RateLimit>,
    /// Octets de corps reçus.
    pub bytes: usize,
}

impl Fetched {
    fn error(error: FetchError) -> Self {
        Self { result: Err(error), pause: None, rate: None, bytes: 0 }
    }
}

#[derive(Debug, Clone)]
pub struct GitHub {
    client: reqwest::Client,
    base: String,
    identity: String,
    authenticated: bool,
}

impl GitHub {
    pub fn new(token: &str) -> anyhow::Result<Self> {
        Self::with_base(GITHUB_API, token)
    }

    /// Client vers une autre base d'API : réservé aux tests, qui simulent GitHub.
    pub fn with_base(base: &str, token: &str) -> anyhow::Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static(MEDIA_TYPE));
        headers.insert("x-github-api-version", HeaderValue::from_static(API_VERSION));
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
        // The cache is scoped to the access identity: a digest prefix, never the token.
        let identity = if token.is_empty() {
            "anonymous".to_owned()
        } else {
            format!("token:{}", &hex::encode(Sha256::digest(token.as_bytes()))[..16])
        };
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_owned(),
            identity,
            authenticated: !token.is_empty(),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Identité d'accès qui cloisonne le cache HTTP.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub fn authenticated(&self) -> bool {
        self.authenticated
    }

    /// GET d'une URL de l'API ; `etag` rend la requête conditionnelle.
    pub async fn fetch(&self, url: &str, etag: Option<&str>) -> Fetched {
        if api_path(url, &self.base).is_none() {
            return Fetched::error(FetchError::Origin);
        }
        let mut request = self.client.get(url);
        if let Some(etag) = etag
            && let Ok(value) = HeaderValue::from_str(etag)
        {
            request = request.header(IF_NONE_MATCH, value);
        }
        self.send(request).await
    }

    /// Requête GraphQL (`POST /graphql`) ; GitHub exige un accès authentifié.
    pub async fn graphql(&self, query: &str, variables: &Value) -> Fetched {
        let body = serde_json::json!({"query": query, "variables": variables});
        self.send(self.client.post(format!("{}/graphql", self.base)).json(&body)).await
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Fetched {
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => return Fetched::error(transport(&error)),
        };
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) if body.len() + chunk.len() <= MAX_BODY => body.extend_from_slice(&chunk),
                Ok(_) => return Fetched::error(FetchError::TooLarge),
                Err(error) => return Fetched::error(transport(&error)),
            }
        }
        let bytes = body.len();
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let rate = rate_limit(&headers);
        let mut pause = None;
        if header("x-ratelimit-remaining") == Some("0") {
            let delay = rate.as_ref().map_or(60.0, |rate| seconds_until(rate.reset).max(60.0));
            pause = Some(delay);
            if matches!(status, 403 | 429) {
                return Fetched { result: Err(FetchError::RateLimited(delay)), pause, rate, bytes };
            }
        }
        if status == 429 || (status == 403 && headers.contains_key("retry-after")) {
            let delay = header("retry-after").unwrap_or("60").parse::<f64>().unwrap_or(60.0);
            return Fetched { result: Err(FetchError::RateLimited(delay)), pause: Some(delay), rate, bytes };
        }
        if status == 304 {
            return Fetched { result: Ok(Response::NotModified), pause, rate, bytes };
        }
        if status != 200 {
            return Fetched { result: Err(FetchError::Status(status)), pause, rate, bytes };
        }
        let result = serde_json::from_slice(&body)
            .map(|payload| Response::Fresh {
                payload,
                link: header("link").unwrap_or_default().to_owned(),
                etag: header("etag").map(str::to_owned),
            })
            .map_err(|_| FetchError::Json);
        Fetched { result, pause, rate, bytes }
    }

    /// État de toutes les ressources de quota (`GET /rate_limit`, non décompté par GitHub).
    pub async fn rate_limits(&self) -> Result<Vec<RateLimit>, FetchError> {
        let Response::Fresh { payload, .. } =
            self.fetch(&format!("{}/rate_limit", self.base), None).await.result?
        else {
            return Err(FetchError::Status(304));
        };
        let resources = payload.get("resources").and_then(Value::as_object).ok_or(FetchError::Json)?;
        Ok(resources.iter().filter_map(|(name, value)| RateLimit::from_resource(name, value)).collect())
    }
}

/// Nom de ressource tel que GitHub les écrit (`core`, `code_search`...) ; tout autre est ignoré.
fn valid_resource(name: &str) -> Option<&str> {
    let valid = !name.is_empty()
        && name.len() <= 40
        && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    valid.then_some(name)
}

fn rate_limit(headers: &HeaderMap) -> Option<RateLimit> {
    let text = |name: &str| headers.get(name)?.to_str().ok().map(str::trim);
    Some(RateLimit {
        resource: valid_resource(text("x-ratelimit-resource").unwrap_or("core"))?.to_owned(),
        remaining: text("x-ratelimit-remaining")?.parse().ok()?,
        limit: text("x-ratelimit-limit")?.parse().ok()?,
        reset: text("x-ratelimit-reset")?.parse().ok()?,
    })
}

fn seconds_until(epoch: i64) -> f64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    // Epoch seconds are exact in an f64 for the next 285 million years.
    #[allow(clippy::cast_precision_loss)]
    let reset = epoch as f64;
    reset - now
}

fn transport(error: &reqwest::Error) -> FetchError {
    if error.is_timeout() { FetchError::TooLarge } else { FetchError::Network }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn github(server: &MockServer) -> GitHub {
        GitHub::with_base(&server.uri(), "test-token").unwrap()
    }

    fn fresh(payload: Value, link: &str, etag: Option<&str>) -> Response {
        Response::Fresh { payload, link: link.into(), etag: etag.map(str::to_owned) }
    }

    #[test]
    fn representation_matches_the_request_headers() {
        assert_eq!(REPRESENTATION, format!("{MEDIA_TYPE}; version={API_VERSION}"));
    }

    #[tokio::test]
    async fn returns_payload_link_etag_and_quota() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/a/b"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id": 1}))
                    .insert_header("link", "<x>; rel=\"next\"")
                    .insert_header("etag", "W/\"abc\"")
                    .insert_header("x-ratelimit-resource", "core")
                    .insert_header("x-ratelimit-limit", "5000")
                    .insert_header("x-ratelimit-remaining", "4999")
                    .insert_header("x-ratelimit-reset", "1900000000"),
            )
            .mount(&server)
            .await;
        let client = github(&server);
        let fetched = client.fetch(&format!("{}/repos/a/b", server.uri()), None).await;
        assert_eq!(fetched.result, Ok(fresh(json!({"id": 1}), "<x>; rel=\"next\"", Some("W/\"abc\""))));
        assert_eq!(fetched.pause, None);
        assert_eq!(
            fetched.rate,
            Some(RateLimit { resource: "core".into(), remaining: 4999, limit: 5000, reset: 1_900_000_000 })
        );
        let request = &server.received_requests().await.unwrap()[0];
        assert_eq!(request.headers["authorization"], "Bearer test-token");
        assert_eq!(request.headers["x-github-api-version"], "2022-11-28");
        assert!(!request.headers.contains_key("if-none-match"));
        assert!(client.authenticated());
        assert!(client.identity().starts_with("token:") && !client.identity().contains("test-token"));
    }

    #[tokio::test]
    async fn conditional_request_can_be_not_modified() {
        let server = MockServer::start().await;
        Mock::given(path("/repos/a/b"))
            .and(header("if-none-match", "W/\"abc\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;
        let fetched = github(&server).fetch(&format!("{}/repos/a/b", server.uri()), Some("W/\"abc\"")).await;
        assert_eq!(fetched.result, Ok(Response::NotModified));
    }

    #[tokio::test]
    async fn refuses_other_origins_without_a_request() {
        let server = MockServer::start().await;
        let fetched = github(&server).fetch("https://api.github.com.evil.test/x", None).await;
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
        let fetched = github(&server).fetch(&format!("{}/big", server.uri()), None).await;
        assert_eq!(fetched.result, Err(FetchError::TooLarge));
    }

    #[tokio::test]
    async fn retry_after_pauses_the_shared_budget() {
        let server = MockServer::start().await;
        Mock::given(path("/limited"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
            .mount(&server)
            .await;
        let fetched = github(&server).fetch(&format!("{}/limited", server.uri()), None).await;
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
                    .insert_header("x-ratelimit-limit", "60")
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
        let forbidden = client.fetch(&format!("{}/forbidden", server.uri()), None).await;
        assert_eq!(forbidden.result, Err(FetchError::RateLimited(60.0)));
        assert_eq!(forbidden.rate.map(|rate| rate.remaining), Some(0));
        // The last allowed answer is kept, and the budget still pauses.
        let last = client.fetch(&format!("{}/last", server.uri()), None).await;
        assert_eq!(last.result, Ok(fresh(json!([]), "", None)));
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
        let gone = client.fetch(&format!("{}/gone", server.uri()), None).await.result.unwrap_err();
        assert_eq!(gone.to_string(), "HTTP 404 : ressource non disponible (aucune suppression inférée)");
        assert!(!gone.is_transient());
        assert_eq!(
            client.fetch(&format!("{}/moved", server.uri()), None).await.result,
            Err(FetchError::Status(301))
        );
        assert_eq!(client.fetch(&format!("{}/html", server.uri()), None).await.result, Err(FetchError::Json));
        assert!(FetchError::Status(503).is_transient() && FetchError::Network.is_transient());
    }

    #[tokio::test]
    async fn rate_limit_endpoint_lists_known_resources() {
        let server = MockServer::start().await;
        Mock::given(path("/rate_limit"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"resources": {
                "core": {"limit": 5000, "remaining": 4990, "reset": 1_900_000_000, "used": 10},
                "graphql": {"limit": 5000, "remaining": 5000, "reset": 1_900_000_100},
                "Bad Name": {"limit": 1, "remaining": 1, "reset": 1},
                "search": {"limit": "30"}
            }})))
            .mount(&server)
            .await;
        let mut limits = github(&server).rate_limits().await.unwrap();
        limits.sort_by(|a, b| a.resource.cmp(&b.resource));
        assert_eq!(
            limits,
            [
                RateLimit { resource: "core".into(), remaining: 4990, limit: 5000, reset: 1_900_000_000 },
                RateLimit { resource: "graphql".into(), remaining: 5000, limit: 5000, reset: 1_900_000_100 },
            ]
        );
    }

    #[tokio::test]
    async fn graphql_posts_the_query_and_reads_its_quota() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(wiremock::matchers::body_partial_json(json!({"variables": {"login": "alice"}})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": {}}))
                    .insert_header("x-ratelimit-resource", "graphql")
                    .insert_header("x-ratelimit-limit", "5000")
                    .insert_header("x-ratelimit-remaining", "4990")
                    .insert_header("x-ratelimit-reset", "1900000000"),
            )
            .mount(&server)
            .await;
        let fetched = github(&server).graphql("query { viewer { login } }", &json!({"login": "alice"})).await;
        assert_eq!(fetched.result, Ok(fresh(json!({"data": {}}), "", None)));
        assert_eq!(fetched.rate.map(|rate| rate.resource), Some("graphql".into()));
        let request = &server.received_requests().await.unwrap()[0];
        assert_eq!(request.headers["authorization"], "Bearer test-token");
    }

    #[test]
    fn anonymous_access_has_its_own_identity() {
        let client = GitHub::with_base("http://127.0.0.1:1", "").unwrap();
        assert_eq!(client.identity(), "anonymous");
        assert!(!client.authenticated());
    }
}
