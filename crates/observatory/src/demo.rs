//! Dossier de démonstration : données fictives pour découvrir l'interface.

use anyhow::bail;
use chrono::{TimeZone, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::store::{Role, ingest_github};

pub async fn create_demo(pool: &PgPool, login: &str) -> anyhow::Result<Uuid> {
    let mut tx = pool.begin().await?;
    let user: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE login = $1")
        .bind(login.to_lowercase())
        .fetch_optional(&mut *tx)
        .await?;
    let Some(user) = user else { bail!("Compte analyste introuvable") };
    let investigation = Uuid::new_v4();
    sqlx::query("INSERT INTO investigations(id, title, description) VALUES($1, $2, $3)")
        .bind(investigation)
        .bind("Terrain de démonstration")
        .bind("Données fictives pour explorer l'interface et la comparaison temporelle.")
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO memberships VALUES($1, $2, 'owner')")
        .bind(investigation)
        .bind(user)
        .execute(&mut *tx)
        .await?;
    for month in [9, 10] {
        let scan = Uuid::new_v4();
        let at = Utc.with_ymd_and_hms(2026, month, 1, 0, 0, 0).single().expect("valid fixed date");
        sqlx::query(
            "INSERT INTO scans(id, investigation_id, requested_by, source, seed, status, idempotency_key, coverage, \
             requested_at, started_at, finished_at) \
             VALUES($1, $2, $3, 'demo', 'demo-lab/observatory', 'succeeded', $4, $5, $6, $6, $6)",
        )
        .bind(scan)
        .bind(investigation)
        .bind(user)
        .bind(Uuid::new_v4())
        .bind(json!({"demo": {"status": "synthetic"}}))
        .bind(at)
        .execute(&mut *tx)
        .await?;
        let repository = json!({"id": "demo:repository:1", "full_name": "demo-lab/observatory",
            "description": "Corpus fictif d'analyse", "private": false,
            "owner": {"id": "demo:account:1", "login": "demo-lab", "type": "Organization"}});
        let root =
            ingest_github(&mut tx, scan, Role::Repository, &repository, "demo:synthetic", at, None).await?;
        for (index, label) in
            ["demo-alice", "demo-bob", "demo-charlie", "demo-delta", "demo-echo"].iter().enumerate()
        {
            let number = index + 2;
            let person = json!({"id": format!("demo:account:{number}"), "login": label, "type": "User"});
            ingest_github(&mut tx, scan, Role::Contributors, &person, "demo:synthetic", at, root).await?;
            let fork = json!({"id": format!("demo:repository:{number}"), "full_name": format!("{label}/observatory"),
                "owner": person, "fork": true, "private": false});
            ingest_github(&mut tx, scan, Role::Forks, &fork, "demo:synthetic", at, root).await?;
        }
        if month == 10 {
            let issue = json!({"id": "demo:issue:1", "number": 12, "title": "Discussion d'architecture",
                "user": {"id": "demo:account:9", "login": "demo-foxtrot"}, "created_at": at.to_rfc3339()});
            ingest_github(&mut tx, scan, Role::Issues, &issue, "demo:synthetic", at, root).await?;
        }
    }
    tx.commit().await?;
    Ok(investigation)
}
