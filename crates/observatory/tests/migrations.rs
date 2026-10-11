//! Migrations : base neuve, base créée par le socle Python 0.1, historique altéré, rôle restreint.

mod common;

use axum::http::StatusCode;
use common::{Client, PASSWORD, app_pool, database, empty_database};
use observatory::migrate::{MIGRATIONS, SchemaError, latest, migrate, verify};
use observatory::security::hash_password;
use serde_json::json;
use sqlx::{Connection, PgConnection};
use uuid::Uuid;

/// Checksum the Python migrator recorded for schema.sql (migration 0001).
const PYTHON_V1_CHECKSUM: &str = "24b37b0857e139ff94d288f977a4f9e02c313530da30fde2424137d913fab7f0";

#[tokio::test]
async fn upgrades_a_database_created_by_the_python_platform() {
    let Some((_, admin)) = empty_database().await else { return };
    let mut conn = PgConnection::connect_with(&admin).await.unwrap();
    // Exactly what investigator/migrate.py did in 0.1.
    sqlx::raw_sql("CREATE TABLE schema_migrations (version integer PRIMARY KEY, checksum text NOT NULL)")
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::raw_sql(MIGRATIONS[0].sql).execute(&mut conn).await.unwrap();
    sqlx::query("INSERT INTO schema_migrations VALUES (1, $1)")
        .bind(PYTHON_V1_CHECKSUM)
        .execute(&mut conn)
        .await
        .unwrap();
    // Rows as the Python ingestion wrote them: the observation repeats the evidence payload.
    let (user, case, scan, entity) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let payload = json!({"id": 123, "full_name": "alice/example", "private": false});
    sqlx::query("INSERT INTO users(id, login, password_hash) VALUES($1, 'alice', $2)")
        .bind(user)
        .bind(hash_password(PASSWORD).unwrap())
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::raw_sql(&format!(
        "INSERT INTO investigations(id, title) VALUES('{case}', 'Avant migration'); \
         INSERT INTO memberships VALUES('{case}', '{user}', 'owner'); \
         INSERT INTO scans(id, investigation_id, requested_by, source, seed, status, idempotency_key) \
           VALUES('{scan}', '{case}', '{user}', 'github', 'alice/example', 'succeeded', '{}'); \
         INSERT INTO evidence(hash, payload, source) VALUES('h1', '{payload}', 'https://api.github.com/repos/alice/example'); \
         INSERT INTO entities(id, kind, external_id, label) VALUES('{entity}', 'Repository', '123', 'alice/example'); \
         INSERT INTO scan_entities(scan_id, entity_id, evidence_hash, data, time_precision) \
           VALUES('{scan}', '{entity}', 'h1', '{payload}', 'instant'); \
         INSERT INTO login_attempts(login) VALUES('alice')",
        Uuid::new_v4()
    ))
    .execute(&mut conn)
    .await
    .unwrap();

    let applied = migrate(&mut conn, Some("ephemeral-tests-only"), None).await.unwrap();
    assert_eq!(applied, (2..=latest()).collect::<Vec<_>>());
    let compacted: bool =
        sqlx::query_scalar("SELECT data IS NULL FROM scan_entities").fetch_one(&mut conn).await.unwrap();
    assert!(compacted, "the duplicated payload is dropped");
    let client_column: String =
        sqlx::query_scalar("SELECT client FROM login_attempts").fetch_one(&mut conn).await.unwrap();
    assert_eq!(client_column, "");

    let pool = app_pool(&admin).await;
    verify(&pool).await.unwrap();
    let mut client = Client::new(&pool);
    client.login("alice").await;
    let (status, graph) = client.get(&format!("/api/v1/scans/{scan}/graph")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(graph["nodes"][0]["data"], payload);
    assert_eq!(graph["nodes"][0]["label"], "alice/example");
    // Running it again is a no-op.
    let again = migrate(&mut conn, Some("ephemeral-tests-only"), None).await.unwrap();
    assert_eq!(again, Vec::<i32>::new());
}

#[tokio::test]
async fn startup_refuses_a_stale_or_altered_schema() {
    let Some((_, admin)) = empty_database().await else { return };
    let mut conn = PgConnection::connect_with(&admin).await.unwrap();
    migrate(&mut conn, Some("ephemeral-tests-only"), Some(1)).await.unwrap();
    let pool = app_pool(&admin).await;
    assert!(matches!(verify(&pool).await, Err(SchemaError::Behind { current: 1, .. })));
    migrate(&mut conn, None, None).await.unwrap();
    verify(&pool).await.unwrap();

    sqlx::query("UPDATE schema_migrations SET checksum = 'edited' WHERE version = 2")
        .execute(&mut conn)
        .await
        .unwrap();
    assert!(matches!(verify(&pool).await, Err(SchemaError::Altered(2))));
    assert!(matches!(migrate(&mut conn, None, None).await, Err(SchemaError::Altered(2))));
    sqlx::query("UPDATE schema_migrations SET checksum = $1 WHERE version = 2")
        .bind(observatory::migrate::checksum(MIGRATIONS[1].sql))
        .execute(&mut conn)
        .await
        .unwrap();

    sqlx::query("INSERT INTO schema_migrations(version, checksum) VALUES(99, 'future')")
        .execute(&mut conn)
        .await
        .unwrap();
    assert!(matches!(verify(&pool).await, Err(SchemaError::Unknown(99))));
    assert!(matches!(migrate(&mut conn, None, None).await, Err(SchemaError::Unknown(99))));
}

#[tokio::test]
async fn the_application_role_cannot_change_the_schema() {
    let Some(db) = database().await else { return };
    let mut conn = db.pool.acquire().await.unwrap();
    for statement in [
        "CREATE TABLE intruder(id integer)",
        "DROP TABLE annotations",
        "ALTER TABLE users ADD COLUMN admin boolean",
        "INSERT INTO schema_migrations(version, checksum) VALUES(50, 'x')",
        "DELETE FROM schema_migrations",
        "TRUNCATE evidence",
    ] {
        let error = sqlx::raw_sql(statement).execute(&mut *conn).await.unwrap_err();
        let code = error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .map(std::borrow::Cow::into_owned);
        assert!(matches!(code.as_deref(), Some("42501" | "42P01")), "{statement}: {error}");
    }
    // Ordinary data access still works.
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&mut *conn).await.unwrap();
    assert_eq!(users, 3);
    let mut admin = db.admin_connection().await;
    let owner: String = sqlx::query_scalar("SELECT tableowner FROM pg_tables WHERE tablename = 'users'")
        .fetch_one(&mut admin)
        .await
        .unwrap();
    assert_ne!(owner, observatory::migrate::APP_ROLE);
    admin.close().await.unwrap();
}
