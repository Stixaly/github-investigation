//! Listes de termes : publication de versions immuables, droits, export et essai.

mod common;

use axum::http::StatusCode;
use common::{Client, database};
use serde_json::{Value, json};

fn example() -> Value {
    serde_json::from_str(include_str!("../../../docs/examples/keyword-list.example.json"))
        .expect("example list")
}

#[tokio::test]
async fn lists_are_published_as_immutable_versions() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Termes").await;
    let url = format!("/api/v1/investigations/{case}/keyword-lists");

    let (status, created) = client.post(&url, json!({"definition": example()})).await;
    assert_eq!((status, created["version"].as_i64()), (StatusCode::CREATED, Some(1)), "{created}");
    let list = created["id"].as_str().unwrap().to_owned();
    assert_eq!(client.post(&url, json!({"definition": example()})).await.0, StatusCode::CONFLICT);

    // A new version keeps the name; its tests must pass.
    let mut changed = example();
    changed["rules"][0]["any"][0]["terms"] = json!(["malware", "ransomware", "stealer", "loader"]);
    let versions = format!("/api/v1/keyword-lists/{list}/versions");
    let (status, published) = client.post(&versions, json!({"definition": changed})).await;
    assert_eq!((status, published["version"].as_i64()), (StatusCode::CREATED, Some(2)));
    let mut renamed = example();
    renamed["name"] = json!("Autre nom");
    let (status, error) = client.post(&versions, json!({"definition": renamed})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error["detail"].as_str().unwrap().contains("ne change pas"));
    let mut failing = example();
    failing["rules"][0]["tests"][0]["expected"] = json!("no_match");
    let (status, error) = client.post(&versions, json!({"definition": failing})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        error["detail"],
        "Tests de la liste en échec — règle « malware-topics », test 1 : no_match attendu, match obtenu"
    );

    let (_, lists) = client.get(&url).await;
    assert_eq!((lists[0]["version"].as_i64(), lists[0]["rules"].as_i64()), (Some(2), Some(3)));
    assert_eq!(lists[0]["published_by"], "alice");
    // The export is importable as is, with its version and status.
    let (_, exported) = client.get(&format!("{versions}/1")).await;
    assert_eq!(
        (exported["definition"]["version"].as_i64(), exported["engine_version"].as_i64()),
        (Some(1), Some(1))
    );
    assert_eq!(exported["definition"]["status"], "published");
    assert_eq!(
        exported["definition"]["rules"][0]["any"][0]["terms"],
        json!(["malware", "ransomware", "stealer"])
    );
    let other = client.investigation("Copie").await;
    let copy = format!("/api/v1/investigations/{other}/keyword-lists");
    assert_eq!(
        client.post(&copy, json!({"definition": exported["definition"]})).await.0,
        StatusCode::CREATED
    );

    // Published versions cannot be altered, even by the application role.
    let altered = sqlx::query("UPDATE keyword_list_versions SET definition = '{}' WHERE list_id = $1::uuid")
        .bind(&list)
        .execute(&db.pool)
        .await;
    assert!(altered.unwrap_err().to_string().contains("immuable"));
    let removed =
        sqlx::query("DELETE FROM collection_profiles WHERE name = 'apercu'").execute(&db.pool).await;
    assert!(removed.is_err());
}

#[tokio::test]
async fn readers_and_outsiders_cannot_publish() {
    let Some(db) = database().await else { return };
    let mut owner = Client::new(&db.pool);
    owner.login("alice").await;
    let case = owner.investigation("Droits").await;
    let url = format!("/api/v1/investigations/{case}/keyword-lists");
    let (_, created) = owner.post(&url, json!({"definition": example()})).await;
    let list = created["id"].as_str().unwrap().to_owned();
    owner
        .put(&format!("/api/v1/investigations/{case}/members"), json!({"login": "reader", "role": "reader"}))
        .await;

    let mut reader = Client::new(&db.pool);
    reader.login("reader").await;
    assert_eq!(reader.get(&url).await.0, StatusCode::OK);
    assert_eq!(reader.get(&format!("/api/v1/keyword-lists/{list}/versions/1")).await.0, StatusCode::OK);
    let versions = format!("/api/v1/keyword-lists/{list}/versions");
    assert_eq!(reader.post(&versions, json!({"definition": example()})).await.0, StatusCode::FORBIDDEN);

    let mut outsider = Client::new(&db.pool);
    outsider.login("bob").await;
    assert_eq!(outsider.get(&url).await.0, StatusCode::NOT_FOUND);
    assert_eq!(outsider.get(&format!("{versions}/1")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(outsider.post(&versions, json!({"definition": example()})).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_list_can_be_tried_without_being_saved() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let (status, preview) = client
        .post(
            "/api/v1/keyword-lists/preview",
            json!({"definition": example(), "input": {
                "repository.name": "demo-stealer", "repository.description": "Tools for malware analysis",
                "repository.topics": ["c2"]}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["failing_tests"], json!([]));
    let outcomes: Vec<(&str, &str)> = preview["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| (result["rule"].as_str().unwrap(), result["outcome"].as_str().unwrap()))
        .collect();
    assert_eq!(
        outcomes,
        [("malware-topics", "match"), ("command-control", "not_evaluated"), ("research-context", "match")]
    );
    let hit = &preview["results"][0]["hits"][0];
    assert_eq!(
        (hit["field"].as_str(), hit["term"].as_str(), hit["start"].as_i64()),
        (Some("repository.name"), Some("stealer"), Some(5))
    );
    let (status, _) = client
        .post(
            "/api/v1/keyword-lists/preview",
            json!({"definition": example(), "input": {"repository.stars": "3"}}),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM keyword_lists").fetch_one(&db.pool).await.unwrap();
    assert_eq!(count, 0, "a preview stores nothing");
}
