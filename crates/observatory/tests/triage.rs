//! Évaluation des listes sur un dossier et file de triage.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use chrono::Utc;
use common::{Client, database};
use observatory::collector::{GitHub, Settings};
use observatory::evaluation;
use observatory::store::{Role, ingest_github};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

fn list() -> Value {
    json!({"format": "observatory.keyword-list", "schema_version": 1, "name": "Démo", "rules": [
        {"id": "observatoire", "name": "Observatoire", "review_priority": "high",
         "any": [{"fields": ["repository.name"], "match": "whole_word", "terms": ["observatory"]}]},
        {"id": "alice", "name": "Alice", "any": [{"fields": ["account.login"], "match": "substring", "terms": ["alice"]}]},
        {"id": "readme", "name": "README", "any": [{"fields": ["repository.readme"], "match": "substring", "terms": ["loader"]}]}
    ]})
}

async fn evaluate(pool: &PgPool) {
    let run = evaluation::claim(pool).await.expect("claim").expect("a queued rule run");
    evaluation::execute(pool, &run).await.expect("evaluation");
}

#[tokio::test]
async fn lists_are_evaluated_on_the_investigation_and_feed_the_triage_queue() {
    let Some(db) = database().await else { return };
    let investigation = observatory::demo::create_demo(&db.pool, "Alice").await.unwrap();
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let (_, created) = client
        .post(&format!("/api/v1/investigations/{investigation}/keyword-lists"), json!({"definition": list()}))
        .await;
    let runs = format!("/api/v1/keyword-lists/{}/versions/1/runs", created["id"].as_str().unwrap());
    let (status, run) = client.post(&runs, json!({})).await;
    assert_eq!((status, run["status"].as_str()), (StatusCode::ACCEPTED, Some("queued")), "{run}");
    assert_eq!(client.post(&runs, json!({})).await.0, StatusCode::CONFLICT, "one pending run per list");
    evaluate(&db.pool).await;

    let (_, history) = client.get(&format!("/api/v1/investigations/{investigation}/rule-runs")).await;
    assert_eq!(history[0]["status"], "succeeded");
    // 6 repositories and 7 accounts; the issue is not evaluated by repository or account rules.
    assert_eq!(history[0]["counts"], json!({"entities": 13, "match": 7, "no_match": 6, "not_evaluated": 6}));

    let triage = format!("/api/v1/investigations/{investigation}/triage");
    let (_, queue) = client.get(&triage).await;
    let items = queue["items"].as_array().unwrap();
    assert_eq!(items.len(), 7);
    assert_eq!(queue["counts"], json!({"new": 7, "in_review": 0, "retained": 0, "dismissed": 0}));
    assert_eq!(
        (items[0]["review_priority"].as_str(), items[0]["rule_id"].as_str()),
        (Some("high"), Some("observatoire"))
    );
    assert_eq!(items[0]["hits"][0]["field"], "repository.name");
    assert_eq!((items[0]["list_version"].as_i64(), items[0]["engine_version"].as_i64()), (Some(1), Some(1)));
    let alice = items.iter().find(|item| item["rule_id"] == "alice").unwrap();
    assert_eq!(
        (alice["entity_label"].as_str(), alice["hits"][0]["excerpt"].as_str()),
        (Some("demo-alice"), Some("demo-alice"))
    );

    // The analyst's decision survives a new run on unchanged data.
    let fork = items.iter().find(|item| item["entity_label"] == "demo-bob/observatory").unwrap();
    let decision = format!("/api/v1/triage/{}", fork["id"].as_str().unwrap());
    let (status, _) =
        client.put(&decision, json!({"state": "dismissed", "note": "Fork de démonstration"})).await;
    assert_eq!(status, StatusCode::OK);
    client.post(&runs, json!({})).await;
    evaluate(&db.pool).await;
    let (_, queue) = client.get(&format!("{triage}?state=dismissed")).await;
    assert_eq!(
        (queue["items"].as_array().unwrap().len(), queue["items"][0]["note"].as_str()),
        (1, Some("Fork de démonstration"))
    );
    assert_eq!(queue["items"][0]["updated_by"], "alice");

    // A different occurrence reopens it.
    let scan: Uuid = sqlx::query_scalar(
        "SELECT id FROM scans WHERE investigation_id = $1 ORDER BY requested_at DESC LIMIT 1",
    )
    .bind(investigation)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let renamed = json!({"id": "demo:repository:3", "full_name": "demo-bob/observatory-ng", "private": false,
                         "owner": {"id": "demo:account:3", "login": "demo-bob"}});
    let mut conn = db.pool.acquire().await.unwrap();
    ingest_github(&mut conn, scan, Role::Repositories, &renamed, "test:renamed", Utc::now(), None)
        .await
        .unwrap();
    client.post(&runs, json!({})).await;
    evaluate(&db.pool).await;
    let (_, queue) = client.get(&format!("{triage}?state=new")).await;
    let reopened = queue["items"].as_array().unwrap().iter().find(|item| item["id"] == fork["id"]).unwrap();
    assert_eq!(reopened["hits"][0]["excerpt"], "observatory-ng");
    assert_eq!(queue["counts"]["dismissed"], 0);
    let (_, everything) = client.get(&triage).await;
    assert_eq!(everything["items"].as_array().unwrap().len(), 7, "no duplicate task");
}

#[tokio::test]
async fn runs_can_be_cancelled_and_access_is_checked() {
    let Some(db) = database().await else { return };
    let investigation = observatory::demo::create_demo(&db.pool, "Alice").await.unwrap();
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let (_, created) = client
        .post(&format!("/api/v1/investigations/{investigation}/keyword-lists"), json!({"definition": list()}))
        .await;
    let list_id = created["id"].as_str().unwrap().to_owned();
    let (_, run) = client.post(&format!("/api/v1/keyword-lists/{list_id}/versions/1/runs"), json!({})).await;
    assert_eq!(
        client.post(&format!("/api/v1/keyword-lists/{list_id}/versions/9/runs"), json!({})).await.0,
        StatusCode::NOT_FOUND
    );
    let cancel = format!("/api/v1/rule-runs/{}/cancel", run["id"].as_str().unwrap());

    client
        .put(
            &format!("/api/v1/investigations/{investigation}/members"),
            json!({"login": "reader", "role": "reader"}),
        )
        .await;
    let mut reader = Client::new(&db.pool);
    reader.login("reader").await;
    assert_eq!(reader.post(&cancel, json!({})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(reader.get(&format!("/api/v1/investigations/{investigation}/triage")).await.0, StatusCode::OK);
    let mut outsider = Client::new(&db.pool);
    outsider.login("bob").await;
    assert_eq!(
        outsider.get(&format!("/api/v1/investigations/{investigation}/triage")).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        outsider.get(&format!("/api/v1/investigations/{investigation}/rule-runs")).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client.get(&format!("/api/v1/investigations/{investigation}/triage?state=maybe")).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    assert_eq!(client.post(&cancel, json!({})).await.0, StatusCode::OK);
    assert!(evaluation::claim(&db.pool).await.unwrap().is_none(), "a cancelled run is not evaluated");
    let (_, history) = client.get(&format!("/api/v1/investigations/{investigation}/rule-runs")).await;
    assert_eq!(history[0]["status"], "cancelled");
}

#[tokio::test]
async fn the_worker_evaluates_queued_lists_when_no_scan_waits() {
    let Some(db) = database().await else { return };
    let investigation = observatory::demo::create_demo(&db.pool, "Alice").await.unwrap();
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let (_, created) = client
        .post(&format!("/api/v1/investigations/{investigation}/keyword-lists"), json!({"definition": list()}))
        .await;
    client
        .post(
            &format!("/api/v1/keyword-lists/{}/versions/1/runs", created["id"].as_str().unwrap()),
            json!({}),
        )
        .await;
    let pool = db.pool.clone();
    let done = async move {
        loop {
            let status: String = sqlx::query_scalar("SELECT status FROM rule_runs LIMIT 1")
                .fetch_one(&pool)
                .await
                .expect("status");
            if status == "succeeded" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let github = GitHub::with_base("http://127.0.0.1:9", "").unwrap();
    tokio::time::timeout(
        Duration::from_secs(30),
        observatory::worker::run(db.pool.clone(), github, Settings::IMMEDIATE, done),
    )
    .await
    .expect("the worker evaluates the list, then stops");
    let items: i64 =
        sqlx::query_scalar("SELECT count(*) FROM triage_items").fetch_one(&db.pool).await.unwrap();
    assert_eq!(items, 7);
}
