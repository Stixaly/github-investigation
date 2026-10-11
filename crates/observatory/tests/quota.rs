//! Collecte économe en quota, contre une API GitHub simulée : cache HTTP,
//! reprise à la page suivante, nouvelles tentatives et rythme partagé.

mod common;

use std::time::Duration;

use chrono::Utc;
use common::{Client, database};
use observatory::collector::budget::{record_rate, status};
use observatory::collector::http::RateLimit;
use observatory::collector::{CORE, CollectError, GitHub, Settings, reserve_request};
use observatory::worker::{claim, execute, run};
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

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

/// Crée un scan de `alice/example`, l'exécute et retourne sa ligne telle que l'API l'expose.
async fn run_scan(
    pool: &PgPool,
    client: &mut Client,
    case: &str,
    github: &GitHub,
    settings: Settings,
) -> Value {
    let scan = client.scan(case, "alice/example").await;
    let job = claim(pool).await.expect("claim").expect("a queued scan");
    assert_eq!(job.id.to_string(), scan);
    execute(pool, github, &job, settings).await.expect("scan result recorded");
    scan_row(client, case, &scan).await
}

async fn scan_row(client: &mut Client, case: &str, scan: &str) -> Value {
    let (_, scans) = client.get(&format!("/api/v1/investigations/{case}/scans")).await;
    scans.as_array().expect("scan list").iter().find(|row| row["id"] == scan).expect("scan row").clone()
}

fn requests_to(requests: &[Request], route: &str) -> usize {
    requests.iter().filter(|request| request.url.path() == route).count()
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned())).fetch_one(pool).await.expect("count")
}

#[tokio::test]
async fn rescans_revalidate_with_etags_then_reuse_fresh_pages() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Cache").await;
    let server = MockServer::start().await;
    let mut routes = vec![
        ("/repos/alice/example".to_owned(), root()),
        ("/repos/alice/example/contributors".to_owned(), json!([{"id": 3, "login": "bob"}])),
    ];
    for route in ["forks", "issues", "events"] {
        routes.push((format!("/repos/alice/example/{route}"), json!([])));
    }
    for (route, body) in routes {
        let etag = format!("W/\"{}\"", route.len());
        Mock::given(path(route.as_str()))
            .and(header("if-none-match", etag.as_str()))
            .respond_with(ResponseTemplate::new(304))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(body).insert_header("etag", etag.as_str()))
            .mount(&server)
            .await;
    }
    let github = GitHub::with_base(&server.uri(), "test-token").unwrap();

    let first = run_scan(&db.pool, &mut client, &case, &github, Settings::IMMEDIATE).await;
    assert_eq!((first["status"].as_str(), first["requests_used"].as_i64()), (Some("succeeded"), Some(5)));
    assert_eq!(first["coverage"]["contributors"]["status"], "complete");
    let evidence = count(&db.pool, "SELECT count(*) FROM evidence").await;
    let (_, graph) = client.get(&format!("/api/v1/scans/{}/graph", first["id"].as_str().unwrap())).await;
    let nodes = graph["nodes"].as_array().unwrap().len();

    // Every page is revalidated: 304 answers are not charged to an authenticated quota.
    let second = run_scan(&db.pool, &mut client, &case, &github, Settings::IMMEDIATE).await;
    assert_eq!((second["status"].as_str(), second["requests_used"].as_i64()), (Some("succeeded"), Some(0)));
    assert_eq!(
        second["coverage"]["stats"],
        json!({"requests": 5, "not_modified": 5, "cache_hits": 0, "retries": 0})
    );
    let (_, graph) = client.get(&format!("/api/v1/scans/{}/graph", second["id"].as_str().unwrap())).await;
    assert_eq!(
        graph["nodes"].as_array().unwrap().len(),
        nodes,
        "a 304 is a new observation of the same data"
    );
    assert_eq!(count(&db.pool, "SELECT count(*) FROM evidence").await, evidence, "same evidence hashes");

    // Within the freshness window, GitHub is not asked at all.
    let fresh = Settings { freshness: Duration::from_secs(3600), ..Settings::IMMEDIATE };
    let third = run_scan(&db.pool, &mut client, &case, &github, fresh).await;
    assert_eq!((third["status"].as_str(), third["requests_used"].as_i64()), (Some("succeeded"), Some(0)));
    assert_eq!(third["coverage"]["stats"]["cache_hits"], 5);
    assert_eq!(server.received_requests().await.unwrap().len(), 10);

    // Another access identity never reads these answers.
    let anonymous = GitHub::with_base(&server.uri(), "").unwrap();
    let fourth = run_scan(&db.pool, &mut client, &case, &anonymous, fresh).await;
    assert_eq!(fourth["requests_used"], 5);
    assert_eq!(fourth["coverage"]["stats"]["cache_hits"], 0);
}

#[tokio::test]
async fn interrupted_scan_resumes_at_its_next_page() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Reprise").await;
    let server = MockServer::start().await;
    respond(&server, "/repos/alice/example", root()).await;
    let next = format!("<{}/repositories/1/contributors?page=2>; rel=\"next\"", server.uri());
    Mock::given(path("/repos/alice/example/contributors"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id": 3, "login": "bob"}]))
                .insert_header("link", next.as_str()),
        )
        .mount(&server)
        .await;
    Mock::given(path("/repositories/1/contributors"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "300"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    respond(&server, "/repositories/1/contributors", json!([{"id": 6, "login": "carol"}])).await;
    for route in ["forks", "issues", "events"] {
        respond(&server, &format!("/repos/alice/example/{route}"), json!([])).await;
    }
    let github = GitHub::with_base(&server.uri(), "").unwrap();
    let scan = client.scan(&case, "alice/example").await;
    let job = claim(&db.pool).await.unwrap().unwrap();
    execute(&db.pool, &github, &job, Settings::IMMEDIATE).await.unwrap();
    let waiting = scan_row(&mut client, &case, &scan).await;
    assert_eq!(waiting["status"], "waiting_quota");
    assert_eq!(waiting["coverage"]["contributors"]["pages"], 1);
    assert!(
        waiting["coverage"]["contributors"].get("reason").is_none(),
        "an interruption is not the page cap"
    );

    // The quota wait is over.
    sqlx::query("UPDATE scans SET available_at = now() WHERE id = $1::uuid")
        .bind(&scan)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE github_rate_limits SET next_request_at = now()").execute(&db.pool).await.unwrap();
    let again = claim(&db.pool).await.unwrap().unwrap();
    assert_eq!(again.id, job.id);
    execute(&db.pool, &github, &again, Settings::IMMEDIATE).await.unwrap();

    let done = scan_row(&mut client, &case, &scan).await;
    assert_eq!((done["status"].as_str(), done["requests_used"].as_i64()), (Some("succeeded"), Some(7)));
    assert_eq!(done["coverage"]["contributors"], json!({"status": "complete", "items": 2, "pages": 2}));
    assert_eq!(done["coverage"]["stats"]["requests"], 7);
    let requests = server.received_requests().await.unwrap();
    // Neither the seed nor the first page is fetched twice.
    assert_eq!(requests_to(&requests, "/repos/alice/example"), 1);
    assert_eq!(requests_to(&requests, "/repos/alice/example/contributors"), 1);
    assert_eq!(requests_to(&requests, "/repositories/1/contributors"), 2);
    let (_, graph) = client.get(&format!("/api/v1/scans/{scan}/graph")).await;
    let labels: Vec<&str> =
        graph["nodes"].as_array().unwrap().iter().map(|node| node["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"bob") && labels.contains(&"carol"));
}

#[tokio::test]
async fn transient_errors_are_retried_within_the_budget() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Erreurs").await;
    let server = MockServer::start().await;
    respond(&server, "/repos/alice/example", root()).await;
    Mock::given(path("/repos/alice/example/contributors"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    respond(&server, "/repos/alice/example/contributors", json!([{"id": 3, "login": "bob"}])).await;
    Mock::given(path("/repos/alice/example/forks"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;
    for route in ["issues", "events"] {
        respond(&server, &format!("/repos/alice/example/{route}"), json!([])).await;
    }
    let github = GitHub::with_base(&server.uri(), "").unwrap();
    let scan = run_scan(&db.pool, &mut client, &case, &github, Settings::IMMEDIATE).await;
    // Seed 1, contributors 1 + 1 retry, forks 1 + 2 retries, issues 1, events 1.
    assert_eq!((scan["status"].as_str(), scan["requests_used"].as_i64()), (Some("partial"), Some(8)));
    assert_eq!(scan["coverage"]["contributors"]["status"], "complete");
    assert_eq!(scan["coverage"]["forks"]["status"], "error");
    assert_eq!(scan["coverage"]["stats"]["retries"], 3);
}

#[tokio::test]
async fn quota_headers_pace_every_worker() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Rythme").await;
    client.scan(&case, "alice/example").await;
    let job = claim(&db.pool).await.unwrap().unwrap();
    let reset = Utc::now().timestamp() + 600;
    let rate = |remaining: i32, reset: i64| RateLimit { resource: CORE.into(), remaining, limit: 60, reset };
    let remaining =
        || count(&db.pool, "SELECT remaining::int8 FROM github_rate_limits WHERE resource = 'core'");

    record_rate(&db.pool, &rate(30, reset), false).await.unwrap();
    reserve_request(&db.pool, &job, CORE, Duration::ZERO).await.unwrap();
    assert_eq!(remaining().await, 29, "each reservation is counted before GitHub answers");
    // A late answer from the same window never raises the estimate...
    record_rate(&db.pool, &rate(40, reset), false).await.unwrap();
    assert_eq!(remaining().await, 29);
    // ...except to give back a request that a 304 did not use.
    record_rate(&db.pool, &rate(35, reset), true).await.unwrap();
    assert_eq!(remaining().await, 30);
    // At the reserve, every worker waits for the reset.
    record_rate(&db.pool, &rate(1, reset), false).await.unwrap();
    let Err(CollectError::RateLimited(wait)) = reserve_request(&db.pool, &job, CORE, Duration::ZERO).await
    else {
        panic!("the reserve must not be spent");
    };
    assert!(wait > 590.0 && wait < 620.0, "{wait}");
    assert!(matches!(
        reserve_request(&db.pool, &job, CORE, Duration::ZERO).await,
        Err(CollectError::RateLimited(_))
    ));
    // A new window replaces the estimate.
    record_rate(&db.pool, &rate(60, reset + 3600), false).await.unwrap();
    assert_eq!(remaining().await, 60);
    assert_eq!(count(&db.pool, "SELECT requests_used::int8 FROM scans").await, 1);
}

#[tokio::test]
async fn github_status_reports_and_records_quotas() {
    let Some(db) = database().await else { return };
    let server = MockServer::start().await;
    let reset = Utc::now().timestamp() + 1000;
    Mock::given(path("/rate_limit"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"resources": {
            "core": {"limit": 5000, "remaining": 4321, "reset": reset, "used": 679},
            "graphql": {"limit": 5000, "remaining": 5000, "reset": reset}
        }})))
        .mount(&server)
        .await;
    let report = status(&db.pool, &GitHub::with_base(&server.uri(), "test-token").unwrap()).await.unwrap();
    assert!(report.starts_with("Accès GitHub : token\n"), "{report}");
    assert!(report.contains("core : 4321/5000 restantes"), "{report}");
    assert!(report.contains("search : pas de fenêtre en cours"), "{report}");
    assert!(report.ends_with("Cache HTTP : 0 réponses, dont 0 pour cet accès"), "{report}");
    assert!(!report.contains("test-token"));
    assert_eq!(
        count(&db.pool, "SELECT remaining::int8 FROM github_rate_limits WHERE resource = 'core'").await,
        4321
    );

    // GitHub unreachable: the stored state is still reported.
    let offline = status(&db.pool, &GitHub::with_base("http://127.0.0.1:9", "").unwrap()).await.unwrap();
    assert!(offline.starts_with("Accès GitHub : anonyme"), "{offline}");
    assert!(
        offline.contains("Relevé GitHub indisponible") && offline.contains("core : 4321/5000"),
        "{offline}"
    );
}

#[tokio::test]
async fn the_worker_purges_expired_cache_entries() {
    let Some(db) = database().await else { return };
    sqlx::query(
        "INSERT INTO http_cache(url, identity, representation, body, validated_at) VALUES \
         ('https://api.github.com/old', 'anonymous', 'x', '[]', now() - interval '8 days'), \
         ('https://api.github.com/recent', 'anonymous', 'x', '[]', now() - interval '6 days')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let github = GitHub::with_base("http://127.0.0.1:9", "").unwrap();
    let shutdown = tokio::time::sleep(Duration::from_millis(300));
    tokio::time::timeout(
        Duration::from_secs(10),
        run(db.pool.clone(), github, Settings::IMMEDIATE, shutdown),
    )
    .await
    .expect("the worker stops when asked");
    let urls: Vec<String> =
        sqlx::query_scalar("SELECT url FROM http_cache").fetch_all(&db.pool).await.unwrap();
    assert_eq!(urls, ["https://api.github.com/recent"]);
}

#[tokio::test]
async fn a_worker_runs_several_scans_at_once() {
    let Some(db) = database().await else { return };
    let mut client = Client::new(&db.pool);
    client.login("alice").await;
    let case = client.investigation("Concurrence").await;
    let server = MockServer::start().await;
    for (id, name) in [(1, "example"), (9, "other")] {
        let seed = json!({"id": id, "full_name": format!("alice/{name}"), "owner": {"id": 2, "login": "alice"},
                          "private": false});
        // A slow seed keeps both scans in flight together.
        Mock::given(path(format!("/repos/alice/{name}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(seed).set_delay(Duration::from_millis(800)),
            )
            .mount(&server)
            .await;
        for route in ["contributors", "forks", "issues", "events"] {
            respond(&server, &format!("/repos/alice/{name}/{route}"), json!([])).await;
        }
        client.scan(&case, &format!("alice/{name}")).await;
    }
    let pool = db.pool.clone();
    let watched = async move {
        let status = |status: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM scans WHERE status = $1")
                    .bind(status)
                    .fetch_one(&pool)
                    .await
                    .expect("scan count")
            }
        };
        while status("running").await < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        while status("succeeded").await < 2 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let github = GitHub::with_base(&server.uri(), "").unwrap();
    let settings = Settings { concurrency: 2, ..Settings::IMMEDIATE };
    tokio::time::timeout(Duration::from_secs(30), run(db.pool.clone(), github, settings, watched))
        .await
        .expect("both scans run together, then the worker stops");
}
