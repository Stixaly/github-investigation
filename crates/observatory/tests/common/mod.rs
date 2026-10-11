//! Base PostgreSQL jetable et client HTTP en mémoire pour les tests d'intégration.
//!
//! Chaque test crée sa propre base (`observatory_test_<uuid>`) sur le serveur
//! désigné par `TEST_DATABASE_URL`, y applique les migrations avec le compte
//! administrateur, puis travaille avec le rôle applicatif `investigator` : les
//! droits restreints de production sont donc ceux que les tests exercent.
//! Aucune base existante n'est vidée.

#![allow(dead_code)]

use std::str::FromStr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use observatory::api::{AppState, router};
use observatory::config::{ApiConfig, Origin};
use observatory::migrate::{APP_ROLE, migrate};
use observatory::users::create_user;
use serde_json::{Value, json};
use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, PgConnection, PgPool};
use tokio::sync::OnceCell;
use tower::ServiceExt;
use uuid::Uuid;

pub const PASSWORD: &str = "local-test-only-password-234!";
const APP_PASSWORD: &str = "ephemeral-tests-only";

static ROLE: OnceCell<()> = OnceCell::const_new();

pub struct TestDb {
    pub name: String,
    /// Connexion administrateur à la base du test.
    pub admin: PgConnectOptions,
    /// Pool du rôle applicatif, comme l'API et le worker.
    pub pool: PgPool,
}

impl TestDb {
    pub async fn admin_connection(&self) -> PgConnection {
        PgConnection::connect_with(&self.admin).await.expect("admin connection")
    }
}

fn server() -> Option<PgConnectOptions> {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        assert!(
            std::env::var_os("REQUIRE_DB_TESTS").is_none(),
            "REQUIRE_DB_TESTS is set: TEST_DATABASE_URL must point to the test-postgres service"
        );
        eprintln!("skipped: TEST_DATABASE_URL is not set");
        return None;
    };
    Some(PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL"))
}

/// Base vide, sans migration. `None` si aucun serveur de test n'est configuré.
pub async fn empty_database() -> Option<(String, PgConnectOptions)> {
    let server = server()?;
    ROLE.get_or_init(|| async {
        // Roles are cluster-wide: create it once, before parallel tests migrate their databases.
        let mut conn = PgConnection::connect_with(&server).await.expect("test server");
        sqlx::raw_sql(&format!(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{APP_ROLE}') THEN \
             CREATE ROLE {APP_ROLE} LOGIN PASSWORD '{APP_PASSWORD}'; END IF; END $$; \
             ALTER ROLE {APP_ROLE} PASSWORD '{APP_PASSWORD}'"
        ))
        .execute(&mut conn)
        .await
        .expect("application role");
    })
    .await;
    let name = format!("observatory_test_{}", Uuid::new_v4().simple());
    let mut conn = PgConnection::connect_with(&server).await.expect("test server");
    sqlx::raw_sql(&format!("CREATE DATABASE {name}")).execute(&mut conn).await.expect("create database");
    let admin = server.database(&name);
    Some((name, admin))
}

/// Base migrée à la dernière version, avec les comptes alice, bob et reader.
pub async fn database() -> Option<TestDb> {
    let (name, admin) = empty_database().await?;
    let mut conn = PgConnection::connect_with(&admin).await.expect("admin connection");
    migrate(&mut conn, Some(APP_PASSWORD), None).await.expect("migrations");
    let pool = app_pool(&admin).await;
    for login in ["alice", "bob", "reader"] {
        create_user(&pool, login, PASSWORD).await.expect("test user");
    }
    Some(TestDb { name, admin, pool })
}

pub async fn app_pool(admin: &PgConnectOptions) -> PgPool {
    let options = admin.clone().username(APP_ROLE).password(APP_PASSWORD);
    observatory::db::pool(options, 5).await.expect("application pool")
}

pub async fn user_id(pool: &PgPool, login: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM users WHERE login = $1")
        .bind(login)
        .fetch_one(pool)
        .await
        .expect("user")
}

/// Client HTTP en mémoire : garde le cookie de session et le jeton CSRF.
pub struct Client {
    router: Router,
    cookie: Option<String>,
    pub csrf: Option<String>,
    /// En-têtes ajoutés à chaque requête.
    pub headers: Vec<(String, String)>,
}

impl Client {
    pub fn new(pool: &PgPool) -> Self {
        Self::with_proxy(pool, false)
    }

    /// `trust_proxy` : l'API lit l'adresse du client dans `X-Real-IP`.
    pub fn with_proxy(pool: &PgPool, trust_proxy: bool) -> Self {
        let config = ApiConfig {
            origin: Origin::parse("http://localhost:8088").expect("origin"),
            trust_proxy_headers: trust_proxy,
            static_dir: "/nonexistent".into(),
            listen: "127.0.0.1:0".parse().expect("address"),
        };
        let router = router(AppState { pool: pool.clone(), config: Arc::new(config) });
        Self { router, cookie: None, csrf: None, headers: Vec::new() }
    }

    pub fn forget_session(&mut self) {
        self.cookie = None;
    }

    pub async fn send(&mut self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.send_with(method, uri, body, &[]).await
    }

    pub async fn send_with(
        &mut self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        extra: &[(&str, &str)],
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(uri);
        if !extra.iter().any(|(name, _)| name.eq_ignore_ascii_case("host")) {
            request = request.header("host", "localhost:8088");
        }
        if let Some(cookie) = &self.cookie {
            request = request.header("cookie", format!("investigation_session={cookie}"));
        }
        if let Some(csrf) = &self.csrf {
            request = request.header("x-csrf-token", csrf);
        }
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        for (name, value) in extra {
            request = request.header(*name, *value);
        }
        let body = match body {
            Some(value) => {
                request = request.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        let response =
            self.router.clone().oneshot(request.body(body).expect("request")).await.expect("response");
        if let Some(cookie) = response.headers().get("set-cookie").and_then(|value| value.to_str().ok()) {
            let value =
                cookie.split(';').next().and_then(|pair| pair.split_once('=')).map(|(_, value)| value);
            self.cookie = value.filter(|_| !cookie.contains("Max-Age=0")).map(str::to_owned);
        }
        let status = response.status();
        let bytes = response.into_body().collect().await.expect("body").to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
        };
        (status, value)
    }

    pub async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.send(Method::GET, uri, None).await
    }

    pub async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send(Method::POST, uri, Some(body)).await
    }

    pub async fn put(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send(Method::PUT, uri, Some(body)).await
    }

    pub async fn delete(&mut self, uri: &str) -> (StatusCode, Value) {
        self.send(Method::DELETE, uri, None).await
    }

    pub async fn login(&mut self, login: &str) {
        let (status, body) = self.post("/api/v1/login", json!({"login": login, "password": PASSWORD})).await;
        assert_eq!(status, StatusCode::OK, "login {login}: {body}");
        self.csrf = body["csrf"].as_str().map(str::to_owned);
    }

    pub async fn investigation(&mut self, title: &str) -> String {
        let (status, body) = self.post("/api/v1/investigations", json!({"title": title})).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["id"].as_str().expect("investigation id").to_owned()
    }

    pub async fn scan(&mut self, investigation: &str, seed: &str) -> String {
        let (status, body) =
            self.post(&format!("/api/v1/investigations/{investigation}/scans"), json!({"seed": seed})).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        body["id"].as_str().expect("scan id").to_owned()
    }
}

/// Dépôt public d'exemple, tel que le renvoie l'API GitHub.
pub fn repo() -> Value {
    json!({"id": 123, "full_name": "alice/example", "private": false,
           "owner": {"id": 42, "login": "alice", "type": "User"}})
}
