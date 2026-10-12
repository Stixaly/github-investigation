//! Profils de collecte, lancements groupés et mesures de référence.

mod common;

use axum::http::StatusCode;
use common::{Client, database};
use observatory::collector::{GitHub, Settings};
use observatory::worker::{claim, execute};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn respond(server: &MockServer, route: &str, body: Value) {
    Mock::given(path(route)).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(server).await;
}

#[tokio::test]
async fn profiles_bound_the_budget_and_the_collection() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let (_, profiles) = client.get("/api/v1/profiles").await;
    let names: Vec<&str> = profiles.as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["apercu", "reseau"]);
    assert_eq!(profiles[0]["settings"]["max_pages"], 1);

    let case = client.investigation("Profils").await;
    let url = format!("/api/v1/investigations/{case}/scans");
    let (status, error) =
        client.post(&url, json!({"seed": "alice/example", "profile": "apercu", "max_requests": 24})).await;
    assert_eq!(
        (status, error["detail"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("Budget : 1 à 12 requêtes pour ce profil"))
    );
    assert_eq!(
        client.post(&url, json!({"seed": "alice/example", "profile": "inconnu"})).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, scan) = client.post(&url, json!({"seed": "alice/example", "profile": "apercu"})).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        (scan["profile_name"].as_str(), scan["profile_version"].as_i64(), scan["max_requests"].as_i64()),
        (Some("apercu"), Some(1), Some(6))
    );

    // The preview profile reads one page of contributors and forks, nothing else.
    let server = MockServer::start().await;
    respond(
        &server,
        "/repos/alice/example",
        json!({"id": 1, "full_name": "alice/example", "private": false,
        "owner": {"id": 2, "login": "alice"}}),
    )
    .await;
    let next = format!("<{}/repositories/1/contributors?page=2>; rel=\"next\"", server.uri());
    Mock::given(path("/repos/alice/example/contributors"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 3, "login": "bob"}]))
                .insert_header("link", next.as_str()),
        )
        .mount(&server)
        .await;
    respond(&server, "/repos/alice/example/forks", json!([])).await;
    let job = claim(&db.pool).await.unwrap().unwrap();
    execute(&db.pool, &GitHub::with_base(&server.uri(), "").unwrap(), &job, Settings::IMMEDIATE)
        .await
        .unwrap();
    let (_, scans) = client.get(&url).await;
    let coverage = &scans[0]["coverage"];
    assert_eq!(scans[0]["status"], "partial");
    assert_eq!(coverage["contributors"]["reason"], "Plafond de 1 page par endpoint atteint");
    assert_eq!(coverage["issues"], json!({"status": "not_requested", "reason": "Hors du profil « apercu »"}));
    assert_eq!(coverage["events"]["status"], "not_requested");
    assert_eq!(scans[0]["requests_used"], 3);
    let requested: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect();
    assert!(!requested.iter().any(|route| route.contains("/issues")
        || route.contains("/events")
        || route.contains("/repositories/")));
}

#[tokio::test]
async fn several_seeds_start_together_once() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Lot").await;
    let url = format!("/api/v1/investigations/{case}/scans/batch");
    let key = Uuid::new_v4();
    let body =
        json!({"seeds": ["alice/one", "github.com/Alice/One", "bob", "carol/two"], "idempotency_key": key});
    let (status, scans) = client.post(&url, body.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{scans}");
    let seeds: Vec<&str> =
        scans.as_array().unwrap().iter().map(|scan| scan["seed"].as_str().unwrap()).collect();
    assert_eq!(seeds, ["alice/one", "bob", "carol/two"], "duplicates are dropped");
    // Replaying the request returns the same scans.
    let (_, again) = client.post(&url, body).await;
    assert_eq!(
        again.as_array().unwrap().iter().map(|scan| &scan["id"]).collect::<Vec<_>>(),
        scans.as_array().unwrap().iter().map(|scan| &scan["id"]).collect::<Vec<_>>()
    );
    let (status, error) = client.post(&url, json!({"seeds": ["alice/one", "not a seed!"]})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error["detail"].as_str().unwrap().starts_with("not a seed! :"));
    // All or nothing: eight more do not fit beside the three pending ones.
    let many: Vec<String> = (0..8).map(|index| format!("dave/repo-{index}")).collect();
    assert_eq!(client.post(&url, json!({"seeds": many})).await.0, StatusCode::TOO_MANY_REQUESTS);
    let (_, all) = client.get(&format!("/api/v1/investigations/{case}/scans")).await;
    assert_eq!(all.as_array().unwrap().len(), 3);
    let eleven: Vec<String> = (0..11).map(|index| format!("erin/repo-{index}")).collect();
    assert_eq!(client.post(&url, json!({"seeds": eleven})).await.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn metrics_describe_the_corpus_without_github() {
    let Some(db) = database().await else { return };
    observatory::demo::create_demo(&db.pool, "Alice").await.unwrap();
    let report = observatory::metrics::report(&db.pool).await.unwrap();
    assert_eq!(report["rows"]["entities"], 14);
    assert!(report["rows"]["evidence"].as_i64().unwrap() > 0);
    assert_eq!(report["recorded_per_day"].as_array().unwrap().len(), 14);
    let today = report["recorded_per_day"][13]["entity_observations"].as_i64().unwrap();
    assert_eq!(
        today,
        report["rows"]["scan_entities"].as_i64().unwrap(),
        "recorded today, whatever the observation date"
    );
    assert_eq!(report["scans"]["by_status"]["succeeded"], 2);
    assert_eq!(report["github"]["requests_sent"], 0);
    assert!(report["database_bytes"].as_i64().unwrap() > 0);
    assert!(report["tables"].as_array().unwrap().iter().any(|table| table["name"] == "scans"));
}
