//! API, sessions, droits, graphe et preuves, contre une vraie base PostgreSQL.

mod common;

use axum::http::{Method, StatusCode};
use chrono::{TimeZone, Utc};
use common::{Client, PASSWORD, database, repo};
use observatory::store::{Role, ingest_github};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn seed_data(
    pool: &PgPool,
    scan: &str,
    when: Option<chrono::DateTime<Utc>>,
    item: &Value,
) -> Option<Uuid> {
    let mut conn = pool.acquire().await.expect("connection");
    let scan = Uuid::parse_str(scan).expect("scan id");
    ingest_github(
        &mut conn,
        scan,
        Role::Repository,
        item,
        "https://api.github.com/repos/alice/example",
        when.unwrap_or_else(Utc::now),
        None,
    )
    .await
    .expect("ingestion")
}

#[tokio::test]
async fn auth_and_csrf() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let csrf = client.csrf.take();
    assert_eq!(
        client.post("/api/v1/investigations", json!({"title": "denied"})).await.0,
        StatusCode::FORBIDDEN
    );
    client.csrf = csrf;
    let (status, _) = client
        .send_with(
            Method::POST,
            "/api/v1/investigations",
            Some(json!({"title": "x"})),
            &[("origin", "https://evil.test")],
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    client.forget_session();
    assert_eq!(client.get("/api/v1/investigations").await.0, StatusCode::UNAUTHORIZED);
    let (status, _) = client
        .send_with(
            Method::POST,
            "/api/v1/login",
            Some(json!({"login": "alice", "password": PASSWORD})),
            &[("origin", "https://evil.test")],
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn session_revocation() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let (status, me) = client.get("/api/v1/me").await;
    assert_eq!((status, me["login"].as_str()), (StatusCode::OK, Some("alice")));
    assert_eq!(client.post("/api/v1/logout", json!({})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(client.get("/api/v1/me").await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn failed_logins_lock_one_client_not_the_account() {
    let Some(db) = database().await else { return };
    let mut client = Client::with_proxy(&db.pool, true);
    let wrong = json!({"login": "alice", "password": "wrong"});
    let right = json!({"login": "alice", "password": PASSWORD});
    for _ in 0..5 {
        let status = client
            .send_with(Method::POST, "/api/v1/login", Some(wrong.clone()), &[("x-real-ip", "10.0.0.1")])
            .await
            .0;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let blocked = client
        .send_with(Method::POST, "/api/v1/login", Some(right.clone()), &[("x-real-ip", "10.0.0.1")])
        .await;
    assert_eq!(blocked.0, StatusCode::TOO_MANY_REQUESTS);
    // The analyst, connecting from elsewhere, is not locked out by someone else's failures.
    let other =
        client.send_with(Method::POST, "/api/v1/login", Some(right), &[("x-real-ip", "10.0.0.2")]).await;
    assert_eq!(other.0, StatusCode::OK);
}

#[tokio::test]
async fn distributed_failures_are_capped_per_account() {
    let Some(db) = database().await else { return };
    let mut client = Client::with_proxy(&db.pool, true);
    for client_number in 0..6 {
        let ip = format!("10.0.1.{client_number}");
        for _ in 0..5 {
            let body = json!({"login": "bob", "password": "wrong"});
            client.send_with(Method::POST, "/api/v1/login", Some(body), &[("x-real-ip", ip.as_str())]).await;
        }
    }
    let body = json!({"login": "bob", "password": PASSWORD});
    let status =
        client.send_with(Method::POST, "/api/v1/login", Some(body), &[("x-real-ip", "10.0.2.1")]).await.0;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn proxy_headers_are_ignored_unless_trusted() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    for index in 0..5 {
        let ip = format!("10.0.3.{index}");
        let body = json!({"login": "alice", "password": "wrong"});
        client.send_with(Method::POST, "/api/v1/login", Some(body), &[("x-real-ip", ip.as_str())]).await;
    }
    let (status, _) = client.post("/api/v1/login", json!({"login": "alice", "password": PASSWORD})).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn private_investigation_is_not_visible() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Investigation test").await;
    let scan = client.scan(&case, "alice/example").await;
    let identity = seed_data(&db.pool, &scan, None, &repo()).await.unwrap();
    let (_, graph) = client.get(&format!("/api/v1/scans/{scan}/graph")).await;
    let proof = graph["nodes"][0]["evidence_hash"].as_str().unwrap().to_owned();
    client.login("bob").await;
    assert_eq!(client.get("/api/v1/investigations").await.1, json!([]));
    assert_eq!(client.get(&format!("/api/v1/scans/{scan}/graph")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(client.get(&format!("/api/v1/evidence/{proof}")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(client.get(&format!("/api/v1/entities/{identity}/overlaps")).await.1, json!([]));
    assert_eq!(client.get(&format!("/api/v1/investigations/{case}/notes")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(client.get(&format!("/api/v1/investigations/{case}/members")).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reader_and_revocation() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Shared").await;
    let members = format!("/api/v1/investigations/{case}/members");
    assert_eq!(client.put(&members, json!({"login": "Reader", "role": "reader"})).await.0, StatusCode::OK);
    assert_eq!(
        client.put(&members, json!({"login": "alice", "role": "reader"})).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        client.put(&members, json!({"login": "nobody", "role": "reader"})).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client.put(&members, json!({"login": "bob", "role": "owner"})).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    client.login("reader").await;
    assert_eq!(client.get("/api/v1/investigations").await.1.as_array().unwrap().len(), 1);
    let scans = format!("/api/v1/investigations/{case}/scans");
    assert_eq!(client.post(&scans, json!({"seed": "alice"})).await.0, StatusCode::FORBIDDEN);
    let notes = format!("/api/v1/investigations/{case}/notes");
    assert_eq!(client.post(&notes, json!({"body": "forbidden"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        client.put(&members, json!({"login": "bob", "role": "analyst"})).await.0,
        StatusCode::FORBIDDEN
    );
    client.login("alice").await;
    assert_eq!(client.delete(&format!("{members}/reader")).await.0, StatusCode::NO_CONTENT);
    client.login("reader").await;
    assert_eq!(client.get(&scans).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn notes_are_kept_per_investigation() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Notes").await;
    let notes = format!("/api/v1/investigations/{case}/notes");
    assert_eq!(client.post(&notes, json!({"body": "Hypothèse"})).await.0, StatusCode::CREATED);
    assert_eq!(client.post(&notes, json!({"body": ""})).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, rows) = client.get(&notes).await;
    assert_eq!((rows[0]["body"].as_str(), rows[0]["login"].as_str()), (Some("Hypothèse"), Some("alice")));
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_events WHERE investigation_id = $1::uuid ORDER BY id")
            .bind(&case)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(actions, ["investigation.create", "note.create"]);
}

#[tokio::test]
async fn idempotency_and_pending_budget() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Budget").await;
    let url = format!("/api/v1/investigations/{case}/scans");
    let body = json!({"seed": "alice/example", "idempotency_key": Uuid::new_v4()});
    let first = client.post(&url, body.clone()).await.1["id"].clone();
    assert_eq!(client.post(&url, body.clone()).await.1["id"], first);
    let mut other = body;
    other["seed"] = json!("other/repo");
    assert_eq!(client.post(&url, other).await.0, StatusCode::CONFLICT);
    client.scan(&case, "github.com/alice/second").await;
    client.scan(&case, "https://github.com/alice").await;
    let seeds: Vec<String> = (0..7).map(|index| format!("alice/batch-{index}")).collect();
    let (status, batch) = client.post(&format!("{url}/batch"), json!({"seeds": seeds})).await;
    assert_eq!((status, batch.as_array().map(Vec::len)), (StatusCode::ACCEPTED, Some(7)));
    let (status, error) = client.post(&url, json!({"seed": "another/repo"})).await;
    assert_eq!(
        (status, error["detail"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("Dix scans au plus en attente ou actifs par dossier"))
    );
}

#[tokio::test]
async fn invalid_requests_are_explained() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Validation").await;
    let url = format!("/api/v1/investigations/{case}/scans");
    let (status, error) = client.post(&url, json!({"seed": "https://evil.example/u/r"})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error["detail"], "Utiliser une URL https://github.com/compte[/depot] sans paramètres");
    assert_eq!(
        client.post(&url, json!({"seed": "a", "max_requests": 51})).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(client.post(&url, json!({"max_requests": 5})).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        client.get("/api/v1/investigations/not-a-uuid/scans").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, _) = client
        .send_with(Method::POST, "/api/v1/investigations", None, &[("content-type", "text/plain")])
        .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let (status, error) = client.get("/api/v1/nothing-here").await;
    assert_eq!((status, error["detail"].as_str()), (StatusCode::NOT_FOUND, Some("Not Found")));
    let (status, _) = client.post("/api/v1/investigations", json!({"title": "x".repeat(70_000)})).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn browser_policy_headers_and_hosts() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    let (status, body) = client.get("/health").await;
    assert_eq!((status, body), (StatusCode::OK, json!({"status": "ok"})));
    assert_eq!(client.get("/ready").await.1, json!({"status": "ready"}));
    let (status, _) = client.send_with(Method::GET, "/health", None, &[("host", "evil.test")]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(client.get("/").await.0, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn canonical_entities_and_temporal_labels() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Temps").await;
    let old = client.scan(&case, "alice/example").await;
    let new = client.scan(&case, "alice/example").await;
    let when = Utc.with_ymd_and_hms(2026, 6, 26, 0, 0, 0).unwrap();
    let first = seed_data(&db.pool, &old, Some(when), &repo()).await;
    seed_data(&db.pool, &old, Some(when), &repo()).await;
    let mut renamed = repo();
    renamed["full_name"] = json!("alice/renamed");
    assert_eq!(seed_data(&db.pool, &new, None, &renamed).await, first);
    let (_, graph) = client.get(&format!("/api/v1/scans/{old}/graph")).await;
    assert_eq!((graph["nodes"].as_array().unwrap().len(), graph["edges"].as_array().unwrap().len()), (2, 1));
    let repository = graph["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "Repository").unwrap();
    assert_eq!(repository["label"], "alice/example");
    assert!(repository["observed_at"].as_str().unwrap().starts_with("2026-06-26"));
    assert_ne!(repository["recorded_at"], repository["observed_at"]);
    assert_eq!(repository["data"]["full_name"], "alice/example");
    let filtered = client.get(&format!("/api/v1/scans/{old}/graph?q=example")).await.1;
    let labels: Vec<&str> =
        filtered["nodes"].as_array().unwrap().iter().map(|n| n["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["alice/example"]);
    assert_eq!(client.get(&format!("/api/v1/scans/{old}/graph?q=renamed")).await.1["nodes"], json!([]));
    // LIKE wildcards typed by the analyst are literal.
    assert_eq!(client.get(&format!("/api/v1/scans/{old}/graph?q=%25")).await.1["nodes"], json!([]));
    assert_eq!(client.get(&format!("/api/v1/scans/{old}/graph?q=_")).await.1["nodes"], json!([]));
}

#[tokio::test]
async fn observations_do_not_copy_their_evidence() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Stockage").await;
    let scan = client.scan(&case, "alice/example").await;
    seed_data(&db.pool, &scan, None, &repo()).await;
    let rows: Vec<(String, bool)> = sqlx::query_as(
        "SELECT e.kind, se.data IS NULL FROM scan_entities se JOIN entities e ON e.id = se.entity_id ORDER BY e.kind",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    // The repository is the evidence itself; its owner is a sub-object and is kept.
    assert_eq!(rows, [("Account".to_owned(), false), ("Repository".to_owned(), true)]);
}

#[tokio::test]
async fn comparison_is_not_deletion() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Comparaison").await;
    let old = client.scan(&case, "alice/example").await;
    let new = client.scan(&case, "alice/example").await;
    seed_data(&db.pool, &old, None, &repo()).await;
    let (_, result) = client.get(&format!("/api/v1/scans/{old}/compare/{new}")).await;
    assert_eq!((result["not_reobserved"].as_i64(), result["newly_observed"].as_i64()), (Some(1), Some(0)));
    assert!(result["notice"].as_str().unwrap().contains("supprimé"));
    client.login("bob").await;
    assert_eq!(client.get(&format!("/api/v1/scans/{old}/compare/{new}")).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn private_payloads_are_never_persisted() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Privé").await;
    let scan = client.scan(&case, "alice/example").await;
    let mut private = repo();
    private["private"] = json!(true);
    assert_eq!(seed_data(&db.pool, &scan, None, &private).await, None);
    let mut internal = repo();
    internal["visibility"] = json!("internal");
    assert_eq!(seed_data(&db.pool, &scan, None, &internal).await, None);
    let mut conn = db.pool.acquire().await.unwrap();
    let event = json!({"public": false, "secret": "private-data"});
    let scan_id = Uuid::parse_str(&scan).unwrap();
    ingest_github(&mut conn, scan_id, Role::Events, &event, "provider", Utc::now(), None).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM evidence").fetch_one(&mut *conn).await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn graph_bounds_and_evidence() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Bornes").await;
    let scan = client.scan(&case, "alice/example").await;
    seed_data(&db.pool, &scan, None, &repo()).await;
    let (_, result) = client.get(&format!("/api/v1/scans/{scan}/graph?limit=1")).await;
    assert_eq!(result["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(result["truncated"], true);
    let proof = result["nodes"][0]["evidence_hash"].as_str().unwrap().to_owned();
    let (status, evidence) = client.get(&format!("/api/v1/evidence/{proof}")).await;
    assert_eq!((status, evidence["payload"]["full_name"].as_str()), (StatusCode::OK, Some("alice/example")));
    let (_, full) = client.get(&format!("/api/v1/scans/{scan}/graph")).await;
    let edge_proof = full["edges"][0]["evidence_hash"].as_str().unwrap().to_owned();
    assert_eq!(client.get(&format!("/api/v1/evidence/{edge_proof}")).await.0, StatusCode::OK);
    assert_eq!(
        client.get(&format!("/api/v1/scans/{scan}/graph?limit=501")).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let focus = full["nodes"][0]["id"].as_str().unwrap().to_owned();
    let (_, around) = client.get(&format!("/api/v1/scans/{scan}/graph?focus={focus}")).await;
    assert_eq!(around["nodes"].as_array().unwrap().len(), 2);
    let overlaps = client.get(&format!("/api/v1/entities/{focus}/overlaps")).await.1;
    assert_eq!(overlaps[0]["title"], "Bornes");
}

#[tokio::test]
async fn investigation_graph_search_and_common_neighbours() {
    let Some(db) = database().await else { return };
    let investigation = observatory::demo::create_demo(&db.pool, "Alice").await.unwrap();
    let mut client = Client::new(&db.pool);
    client.login("alice").await;

    // Both demo scans in one view: September's entities are all observed again in October.
    let (status, graph) = client.get(&format!("/api/v1/investigations/{investigation}/graph")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (graph["temporal_mode"].as_str(), graph["scan_count"].as_i64()),
        (Some("investigation"), Some(2))
    );
    let nodes = graph["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 14);
    let (_, first) = client.get(&format!("/api/v1/investigations/{investigation}/graph?limit=1")).await;
    assert_eq!(first["nodes"][0]["label"], "demo-lab/observatory", "the seed is selected first");
    assert_eq!(first["truncated"], true);
    let id = |label: &str| {
        nodes.iter().find(|node| node["label"] == label).unwrap()["id"].as_str().unwrap().to_owned()
    };
    let (alice, bob) = (id("demo-alice"), id("demo-bob"));
    let (_, focused) =
        client.get(&format!("/api/v1/investigations/{investigation}/graph?focus={alice}&limit=50")).await;
    let focused: Vec<&str> =
        focused["nodes"].as_array().unwrap().iter().map(|node| node["label"].as_str().unwrap()).collect();
    assert!(focused.contains(&"demo-alice/observatory") && !focused.contains(&"demo-bob"));

    // Search across the analyst's investigations, closest names first.
    let (status, found) = client.get("/api/v1/search?q=alice").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found[0]["label"], "demo-alice");
    assert_eq!(
        found[0]["investigations"],
        json!([{"id": investigation, "title": "Terrain de démonstration"}])
    );
    assert!(found.as_array().unwrap().iter().any(|entity| entity["label"] == "demo-alice/observatory"));
    assert_eq!(client.get("/api/v1/search?q=%25").await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(client.get("/api/v1/search?q=a_").await.1, json!([]), "wildcards are taken literally");

    // Two contributors share the demo repository.
    let (status, common) =
        client.get(&format!("/api/v1/investigations/{investigation}/common?a={alice}&b={bob}")).await;
    assert_eq!(status, StatusCode::OK);
    let shared = common["common"].as_array().unwrap();
    assert_eq!(shared.len(), 1);
    assert_eq!(
        (&shared[0]["label"], &shared[0]["with_a"], &shared[0]["with_b"]),
        (&json!("demo-lab/observatory"), &json!(["CONTRIBUTED_TO"]), &json!(["CONTRIBUTED_TO"]))
    );
    let same = format!("/api/v1/investigations/{investigation}/common?a={alice}&b={alice}");
    assert_eq!(client.get(&same).await.0, StatusCode::UNPROCESSABLE_ENTITY);

    // Outside the investigation, nothing is found and nothing is shown.
    let mut outsider = Client::new(&db.pool);
    outsider.login("bob").await;
    assert_eq!(outsider.get("/api/v1/search?q=alice").await.1, json!([]));
    assert_eq!(
        outsider.get(&format!("/api/v1/investigations/{investigation}/graph")).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        outsider.get(&format!("/api/v1/investigations/{investigation}/common?a={alice}&b={bob}")).await.0,
        StatusCode::NOT_FOUND
    );
}
