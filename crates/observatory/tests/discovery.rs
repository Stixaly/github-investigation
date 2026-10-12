//! Découverte GitHub contre une API de recherche simulée : plan, subdivision,
//! candidats, recherche de code et droits.

mod common;

use axum::http::StatusCode;
use chrono::Utc;
use common::{Client, database};
use observatory::collector::{GitHub, Settings};
use observatory::worker::{claim, execute};
use observatory_domain::discovery::{FIRST_DAY, date};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::matchers::{header, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn repository(id: u64, name: &str) -> Value {
    json!({"id": id, "full_name": name, "private": false, "fork": false, "description": null,
           "owner": {"id": id + 1000, "login": name.split('/').next(), "type": "User"}})
}

async fn search(server: &MockServer, route: &str, q: &str, total: u64, items: Value, resource: &str) {
    Mock::given(path(route))
        .and(query_param("q", q))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"total_count": total, "incomplete_results": false, "items": items}))
                .insert_header("x-ratelimit-resource", resource)
                .insert_header("x-ratelimit-limit", "30")
                .insert_header("x-ratelimit-remaining", "28")
                .insert_header("x-ratelimit-reset", (Utc::now().timestamp() + 60).to_string().as_str()),
        )
        .mount(server)
        .await;
}

async fn launch(client: &mut Client, case: &str, body: Value) -> (StatusCode, Value) {
    client.post(&format!("/api/v1/investigations/{case}/discoveries"), body).await
}

async fn run(db: &common::TestDb, server: &MockServer, token: &str) {
    let job = claim(&db.pool).await.expect("claim").expect("a queued discovery");
    let github = GitHub::with_base(&server.uri(), token).expect("client");
    execute(&db.pool, &github, &job, Settings::IMMEDIATE).await.expect("result recorded");
}

#[tokio::test]
async fn saturated_searches_are_subdivided_then_marked_partial() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Découverte").await;
    let (status, scan) = launch(
        &mut client,
        &case,
        json!({"kind": "repositories", "query": "loader", "max_pages": 1, "max_subdivisions": 1, "max_requests": 6}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{scan}");
    assert_eq!(
        (scan["source"].as_str(), scan["seed"].as_str()),
        (Some("search"), Some("Dépôts : loader fork:true"))
    );

    let server = MockServer::start().await;
    let today = Utc::now().timestamp().div_euclid(86_400);
    let middle = FIRST_DAY + (today - FIRST_DAY) / 2;
    let early = format!("loader fork:true created:{}..{}", date(FIRST_DAY), date(middle));
    let late = format!("loader fork:true created:{}..{}", date(middle + 1), date(today));
    search(
        &server,
        "/search/repositories",
        "loader fork:true",
        5000,
        json!([repository(1, "alice/loader")]),
        "search",
    )
    .await;
    search(&server, "/search/repositories", &early, 1, json!([repository(2, "bob/old-loader")]), "search")
        .await;
    search(
        &server,
        "/search/repositories",
        &late,
        3000,
        json!([repository(3, "carol/new-loader")]),
        "search",
    )
    .await;
    run(&db, &server, "").await;

    let (_, scans) = client.get(&format!("/api/v1/investigations/{case}/scans")).await;
    let coverage = &scans[0]["coverage"];
    assert_eq!((scans[0]["status"].as_str(), scans[0]["requests_used"].as_i64()), (Some("partial"), Some(3)));
    assert_eq!(coverage["q1"]["status"], "subdivided");
    assert_eq!(coverage["q1"]["reason"], "5000 résultats : subdivisée en deux requêtes");
    assert_eq!(
        (coverage["q2"]["status"].as_str(), coverage["q2"]["query"].as_str()),
        (Some("complete"), Some(early.as_str()))
    );
    assert_eq!(coverage["q3"]["reason"], "3000 résultats : seuls les 100 premiers sont accessibles");
    assert_eq!(coverage["steps"].as_array().unwrap().len(), 3);
    let (_, graph) = client.get(&format!("/api/v1/scans/{}/graph", scan["id"].as_str().unwrap())).await;
    let mut labels: Vec<&str> =
        graph["nodes"].as_array().unwrap().iter().map(|node| node["label"].as_str().unwrap()).collect();
    labels.sort_unstable();
    assert_eq!(labels, ["alice", "alice/loader", "bob", "bob/old-loader", "carol", "carol/new-loader"]);
    let remaining: Option<i32> =
        sqlx::query_scalar("SELECT remaining FROM github_rate_limits WHERE resource = 'search'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(remaining.is_some(), "the search quota is paced on its own resource");
}

#[tokio::test]
async fn code_search_needs_a_token_and_keeps_no_fragment() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Code").await;
    let body = json!({"kind": "code", "query": "\"VirtualAllocEx\" language:C", "include_forks": false});
    launch(&mut client, &case, body.clone()).await;
    let server = MockServer::start().await;
    run(&db, &server, "").await;
    let (_, scans) = client.get(&format!("/api/v1/investigations/{case}/scans")).await;
    assert_eq!(scans[0]["status"], "failed");
    assert_eq!(scans[0]["error"], "La recherche de code exige un token GitHub configuré sur le worker");
    assert!(server.received_requests().await.unwrap().is_empty());

    let mut second = body;
    second["idempotency_key"] = json!(Uuid::new_v4());
    let (_, scan) = launch(&mut client, &case, second).await;
    let hit = json!({"name": "inject.c", "path": "src/inject.c", "sha": "abc123", "score": 1.0,
                     "repository": repository(7, "dave/no-description"),
                     "text_matches": [{"fragment": "VirtualAllocEx(process, ...)"}]});
    Mock::given(path("/search/code"))
        .and(query_param("q", "\"VirtualAllocEx\" language:C"))
        .and(header("accept", "application/vnd.github+json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"total_count": 1, "incomplete_results": false, "items": [hit]}))
                .insert_header("x-ratelimit-resource", "code_search")
                .insert_header("x-ratelimit-limit", "10")
                .insert_header("x-ratelimit-remaining", "9")
                .insert_header("x-ratelimit-reset", (Utc::now().timestamp() + 60).to_string().as_str()),
        )
        .mount(&server)
        .await;
    run(&db, &server, "test-token").await;
    let (_, scans) = client.get(&format!("/api/v1/investigations/{case}/scans")).await;
    let row = scans.as_array().unwrap().iter().find(|row| row["id"] == scan["id"]).unwrap();
    assert_eq!(row["status"], "succeeded", "{row}");
    let (_, graph) = client.get(&format!("/api/v1/scans/{}/graph", scan["id"].as_str().unwrap())).await;
    let repository =
        graph["nodes"].as_array().unwrap().iter().find(|node| node["kind"] == "Repository").unwrap();
    assert_eq!(repository["label"], "dave/no-description");
    let (_, proof) =
        client.get(&format!("/api/v1/evidence/{}", repository["evidence_hash"].as_str().unwrap())).await;
    assert_eq!(proof["payload"]["path"], "src/inject.c");
    assert!(
        proof["payload"].get("text_matches").is_none(),
        "fragments stay out until content acquisition is allowed"
    );
    let remaining: Option<i32> =
        sqlx::query_scalar("SELECT remaining FROM github_rate_limits WHERE resource = 'code_search'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(remaining.is_some_and(|remaining| remaining <= 9));
}

#[tokio::test]
async fn plans_are_previewed_without_github_and_launched_by_writers() {
    let Some(db) = database().await else { return };
    let mut owner = Client::new(&db.pool);
    owner.login("alice").await;
    let case = owner.investigation("Plan").await;
    let example: Value =
        serde_json::from_str(include_str!("../../../docs/examples/keyword-list.example.json")).unwrap();
    let (_, list) = owner
        .post(&format!("/api/v1/investigations/{case}/keyword-lists"), json!({"definition": example}))
        .await;
    let choice = json!({"kind": "repositories", "list": {"id": list["id"], "version": 1}});
    let preview_url = format!("/api/v1/investigations/{case}/discoveries/preview");
    let (status, preview) = owner.post(&preview_url, choice.clone()).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let queries: Vec<&str> = preview["plan"]["queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|query| query["q"].as_str().unwrap())
        .collect();
    assert_eq!(queries[0], "\"malware\" in:name,description,topics fork:true");
    assert_eq!(queries.len(), 3 + 2 + 3, "one query per term of the positive clauses");
    assert_eq!(preview["label"], "Dépôts : liste « Thèmes malware — aide au triage » v1");
    assert_eq!(preview["max_requests_upper_bound"], (8 + 2 * 4) * 2);
    assert_eq!(preview["resource"], "search");
    let (status, error) =
        owner.post(&preview_url, json!({"kind": "code", "list": {"id": list["id"], "version": 1}})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error["detail"].as_str().unwrap().contains("recherche de code part d'une requête"));
    assert_eq!(
        owner.post(&preview_url, json!({"kind": "repositories"})).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let missing = json!({"kind": "repositories", "list": {"id": Uuid::new_v4(), "version": 1}});
    assert_eq!(owner.post(&preview_url, missing).await.0, StatusCode::NOT_FOUND);

    owner
        .put(&format!("/api/v1/investigations/{case}/members"), json!({"login": "reader", "role": "reader"}))
        .await;
    let mut reader = Client::new(&db.pool);
    reader.login("reader").await;
    assert_eq!(reader.post(&preview_url, choice.clone()).await.0, StatusCode::OK);
    assert_eq!(launch(&mut reader, &case, choice.clone()).await.0, StatusCode::FORBIDDEN);
    let mut outsider = Client::new(&db.pool);
    outsider.login("bob").await;
    assert_eq!(outsider.post(&preview_url, choice.clone()).await.0, StatusCode::NOT_FOUND);

    let mut keyed = choice;
    keyed["idempotency_key"] = json!(Uuid::new_v4());
    let (status, first) = launch(&mut owner, &case, keyed.clone()).await;
    assert_eq!((status, first["plan"]["queries"].as_array().map(Vec::len)), (StatusCode::ACCEPTED, Some(8)));
    assert_eq!(launch(&mut owner, &case, keyed.clone()).await.1["id"], first["id"]);
    keyed["max_requests"] = json!(24);
    assert_eq!(launch(&mut owner, &case, keyed).await.0, StatusCode::CONFLICT);
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_events WHERE action = 'discovery.create'")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(actions.len(), 1);
}
