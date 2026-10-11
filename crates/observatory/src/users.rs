//! Comptes locaux, créés par l'administrateur via la CLI.

use anyhow::anyhow;
use observatory_domain::text::{check_new_password, normalize_login};
use sqlx::PgPool;
use uuid::Uuid;

use crate::security::hash_password;

pub async fn create_user(pool: &PgPool, login: &str, password: &str) -> anyhow::Result<Uuid> {
    let login = normalize_login(login)?;
    check_new_password(password)?;
    let candidate = password.to_owned();
    let hash = tokio::task::spawn_blocking(move || hash_password(&candidate))
        .await
        .map_err(|error| anyhow!("hashing task: {error}"))??;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id, login, password_hash) VALUES($1, $2, $3)")
        .bind(id)
        .bind(&login)
        .bind(hash)
        .execute(pool)
        .await
        .map_err(|error| match &error {
            sqlx::Error::Database(db) if db.is_unique_violation() => anyhow!("Le compte {login} existe déjà"),
            _ => error.into(),
        })?;
    Ok(id)
}
