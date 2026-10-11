//! Collecte contre une API GitHub simulée, file de scans et import historique.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::{Client, database};
use observatory::collector::{CORE, CollectError, GitHub, Settings, guard, reserve_request};
use observatory::legacy::{ImportRequest, import_snapshot};
use observatory::worker::{claim, execute, release, run};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn root() -> Value {
    json!({"id": 1, "full_name": "alice/example", "owner": {"id": 2, "login": "alice"}, "private": false})
}

async fn respond(server: &MockServer, route: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn offline_full_collection() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Collecte").await;
    let scan = client.scan(&case, "https://github.com/alice/example").await;
    let server = MockServer::start().await;
    respond(&server, "/repos/alice/example", root()).await;
    // Page 1 announces page 2 on the canonical /repositories/<id>/ path, as GitHub does.
    let next = format!("<{}/repositories/1/contributors?page=2>; rel=\"next\"", server.uri());
    Mock::given(path("/repos/alice/example/contributors"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 3, "login": "bob"}]))
                .insert_header("link", next.as_str()),
        )
        .mount(&server)
        .await;
    respond(&server, "/repositories/1/contributors", json!([{"id": 6, "login": "carol"}])).await;
    respond(
        &server,
        "/repos/alice/example/issues",
        json!([{"id": 4, "number": 1, "title": "An issue", "user": {"id": 3, "login": "bob"}}]),
    )
    .await;
    respond(
        &server,
        "/repos/alice/example/forks",
        json!([{"id": 5, "full_name": "bob/example", "private": false, "owner": {"id": 3, "login": "bob"}}]),
    )
    .await;
    respond(&server, "/repos/alice/example/events", json!([
        {"id": "1234", "public": true, "type": "WatchEvent", "actor": {"id": 3, "login": "bob"}, "created_at": "2026-01-01T00:00:00Z"},
        {"id": "1235", "public": false, "type": "WatchEvent", "actor": {"id": 7, "login": "hidden"}}
    ])).await;

    let job = claim(&db.pool).await.unwrap().unwrap();
    assert_eq!(job.id.to_string(), scan);
    let github = GitHub::with_base(&server.uri(), "").unwrap();
    execute(&db.pool, &github, &job, Settings::IMMEDIATE).await.unwrap();

    let (_, scans) = client.get(&format!("/api/v1/investigations/{case}/scans")).await;
    assert_eq!(scans[0]["status"], "succeeded");
    assert_eq!(scans[0]["requests_used"], 6);
    let (_, graph) = client.get(&format!("/api/v1/scans/{scan}/graph")).await;
    let mut kinds: Vec<&str> =
        graph["edges"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(kinds, ["AUTHORED", "CONTRIBUTED_TO", "FORK_OF", "IN_REPOSITORY", "OWNS", "STARRED"]);
    let starred = graph["edges"].as_array().unwrap().iter().find(|e| e["kind"] == "STARRED").unwrap();
    assert!(starred["source_event_at"].as_str().unwrap().starts_with("2026-01-01"));
    let coverage = &graph["coverage"];
    assert_eq!(
        (coverage["events"]["status"].as_str(), coverage["contributors"]["pages"].as_i64()),
        (Some("windowed"), Some(2))
    );
    let labels: Vec<&str> =
        graph["nodes"].as_array().unwrap().iter().map(|n| n["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"carol") && !labels.contains(&"hidden"));
}

#[tokio::test]
async fn foreign_pagination_marks_the_endpoint_in_error() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Pagination").await;
    let scan = client.scan(&case, "alice/example").await;
    let server = MockServer::start().await;
    respond(&server, "/repos/alice/example", root()).await;
    Mock::given(path("/repos/alice/example/contributors"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([]))
                .insert_header("link", "<https://evil.test/x>; rel=\"next\""),
        )
        .mount(&server)
        .await;
    for route in ["forks", "issues", "events"] {
        respond(&server, &format!("/repos/alice/example/{route}"), json!([])).await;
    }
    let job = claim(&db.pool).await.unwrap().unwrap();
    execute(&db.pool, &GitHub::with_base(&server.uri(), "").unwrap(), &job, Settings::IMMEDIATE)
        .await
        .unwrap();
    let (_, graph) = client.get(&format!("/api/v1/scans/{scan}/graph")).await;
    assert_eq!(graph["coverage"]["contributors"]["status"], "error");
    assert_eq!(graph["coverage"]["contributors"]["reason"], "Lien de pagination non autorisé");
    let (_, scans) = client.get(&format!("/api/v1/investigations/{case}/scans")).await;
    assert_eq!(scans[0]["status"], "partial");
}

#[tokio::test]
async fn budget_and_quota_are_persisted() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Quota").await;
    let scan = client.scan(&case, "alice/example").await;
    let job = claim(&db.pool).await.unwrap().unwrap();
    reserve_request(&db.pool, &job, CORE, Duration::from_secs(1)).await.unwrap();
    assert!(matches!(
        reserve_request(&db.pool, &job, CORE, Duration::from_secs(1)).await,
        Err(CollectError::RateLimited(_))
    ));
    sqlx::query("UPDATE scans SET requests_used = max_requests WHERE id = $1::uuid")
        .bind(&scan)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(matches!(
        reserve_request(&db.pool, &job, CORE, Duration::ZERO).await,
        Err(CollectError::BudgetExceeded)
    ));
}

#[tokio::test]
async fn rate_limit_requeues_without_losing_observations() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Attente").await;
    let scan = client.scan(&case, "alice/example").await;
    let server = MockServer::start().await;
    respond(&server, "/repos/alice/example", root()).await;
    Mock::given(path("/repos/alice/example/contributors"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "300"))
        .mount(&server)
        .await;
    let job = claim(&db.pool).await.unwrap().unwrap();
    execute(&db.pool, &GitHub::with_base(&server.uri(), "").unwrap(), &job, Settings::IMMEDIATE)
        .await
        .unwrap();
    let (status, waits): (String, bool) = sqlx::query_as(
        "SELECT status, available_at > requested_at + interval '299 seconds' FROM scans WHERE id = $1::uuid",
    )
    .bind(&scan)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((status.as_str(), waits), ("waiting_quota", true));
    let paused: bool =
        sqlx::query_scalar(
            "SELECT next_request_at > now() + interval '200 seconds' FROM github_rate_limits WHERE resource = 'core'",
        )
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(paused, "every worker must wait for the shared quota");
    assert_eq!(
        client.get(&format!("/api/v1/scans/{scan}/graph")).await.1["nodes"].as_array().unwrap().len(),
        2
    );
    assert!(claim(&db.pool).await.unwrap().is_none(), "a waiting scan is not claimed before its time");
}

#[tokio::test]
async fn worker_fencing_cancel_and_release() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Lease").await;
    let scan = client.scan(&case, "alice/example").await;
    let job = claim(&db.pool).await.unwrap().unwrap();
    assert_eq!(job.id.to_string(), scan);
    assert!(claim(&db.pool).await.unwrap().is_none());
    // A worker stopping mid-scan hands it back at once, and loses its write access.
    release(&db.pool, &job).await.unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    assert!(matches!(guard(&mut conn, &job).await, Err(CollectError::LeaseLost)));
    let again = claim(&db.pool).await.unwrap().unwrap();
    assert_eq!((again.id, again.attempts), (job.id, 1));
    assert_eq!(client.post(&format!("/api/v1/scans/{scan}/cancel"), json!({})).await.0, StatusCode::OK);
    assert!(matches!(guard(&mut conn, &again).await, Err(CollectError::LeaseLost)));
}

#[tokio::test]
async fn legacy_import_is_idempotent_and_inert() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Historique").await;
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("users_db.json"),
        r#"{"alice": {"id": 42, "login": "alice", "type": "User"}, "ghost": {"error": "404"}}"#,
    )
    .unwrap();
    std::fs::write(
        directory.path().join("repos_db.json"),
        r#"{"alice": [{"full_name": "alice/old", "fork": false}, {"full_name": "alice/secret", "private": true}], "ghost": {"error": "404"}}"#,
    )
    .unwrap();
    std::fs::write(directory.path().join("sets.pkl"), b"not-a-pickle-never-read").unwrap();
    let request = ImportRequest {
        directory: directory.path().to_owned(),
        investigation: Uuid::parse_str(&case).unwrap(),
        login: "Alice".into(),
        observed_date: chrono::NaiveDate::from_ymd_opt(2026, 6, 26),
    };
    let first = import_snapshot(&db.pool, &request).await.unwrap();
    assert_eq!(
        (
            first.counters.users,
            first.counters.repositories,
            first.counters.user_errors,
            first.counters.skipped
        ),
        (1, 1, 1, 1)
    );
    let again = import_snapshot(&db.pool, &request).await.unwrap();
    assert!(again.reused);
    assert_eq!(again.scan_id, first.scan_id);
    let (_, graph) = client.get(&format!("/api/v1/scans/{}/graph", first.scan_id)).await;
    assert_eq!(graph["edges"].as_array().unwrap().len(), 1);
    let repository = graph["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "Repository").unwrap();
    assert_eq!(repository["provisional"], true);
    assert!(graph["nodes"].as_array().unwrap().iter().all(|n| n["time_precision"] == "day"));
    // A reader cannot import into the investigation.
    let reader = ImportRequest { login: "reader".into(), ..request };
    assert!(import_snapshot(&db.pool, &reader).await.is_err());
}

async fn scan_status(pool: &sqlx::PgPool, scan: &str) -> (String, i32) {
    sqlx::query_as("SELECT status, generation FROM scans WHERE id = $1::uuid")
        .bind(scan)
        .fetch_one(pool)
        .await
        .expect("scan status")
}

#[tokio::test]
async fn worker_loop_runs_queued_scans_until_shutdown() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Boucle").await;
    let scan = client.scan(&case, "alice/example").await;
    let server = MockServer::start().await;
    respond(&server, "/repos/alice/example", root()).await;
    for route in ["contributors", "forks", "issues", "events"] {
        respond(&server, &format!("/repos/alice/example/{route}"), json!([])).await;
    }
    let pool = db.pool.clone();
    let watched = scan.clone();
    let finished = async move {
        while scan_status(&pool, &watched).await.0 != "succeeded" {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let github = GitHub::with_base(&server.uri(), "").unwrap();
    tokio::time::timeout(
        Duration::from_secs(30),
        run(db.pool.clone(), github, Settings::IMMEDIATE, finished),
    )
    .await
    .expect("the worker stops when asked");
    assert_eq!(scan_status(&db.pool, &scan).await.0, "succeeded");
}

#[tokio::test]
async fn stopping_mid_scan_hands_it_back() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Arrêt").await;
    let scan = client.scan(&case, "alice/example").await;
    let server = MockServer::start().await;
    Mock::given(path("/repos/alice/example"))
        .respond_with(ResponseTemplate::new(200).set_body_json(root()).set_delay(Duration::from_secs(20)))
        .mount(&server)
        .await;
    let github = GitHub::with_base(&server.uri(), "").unwrap();
    let shutdown = tokio::time::sleep(Duration::from_millis(500));
    tokio::time::timeout(
        Duration::from_secs(10),
        run(db.pool.clone(), github, Settings::IMMEDIATE, shutdown),
    )
    .await
    .expect("shutdown does not wait for GitHub");
    // Claimed (generation 1), then released at once (generation 2) for another worker.
    assert_eq!(scan_status(&db.pool, &scan).await, ("queued".to_owned(), 2));
    assert!(claim(&db.pool).await.unwrap().is_some());
}

#[tokio::test]
async fn demo_investigation_is_browsable() {
    let Some(db) = database().await else { return };
    let investigation = observatory::demo::create_demo(&db.pool, "Alice").await.unwrap();
    assert!(observatory::demo::create_demo(&db.pool, "nobody").await.is_err());
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let (_, list) = client.get("/api/v1/investigations").await;
    assert_eq!(
        (list[0]["title"].as_str(), list[0]["role"].as_str()),
        (Some("Terrain de démonstration"), Some("owner"))
    );
    let (_, scans) = client.get(&format!("/api/v1/investigations/{investigation}/scans")).await;
    let (october, september) = (scans[0]["id"].as_str().unwrap(), scans[1]["id"].as_str().unwrap());
    let (_, graph) = client.get(&format!("/api/v1/scans/{october}/graph")).await;
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 14);
    let organization = graph["nodes"].as_array().unwrap().iter().find(|n| n["label"] == "demo-lab").unwrap();
    assert_eq!(organization["data"]["type"], "Organization");
    let (_, comparison) = client.get(&format!("/api/v1/scans/{september}/compare/{october}")).await;
    assert_eq!(
        (comparison["newly_observed"].as_i64(), comparison["not_reobserved"].as_i64()),
        (Some(2), Some(0))
    );
}
