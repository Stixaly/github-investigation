//! Connexions PostgreSQL : pool borné, fuseau UTC et délais de requête.

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};

use crate::config::{secret, var_or};

#[derive(Debug, Clone)]
pub struct DbSettings {
    pub host: String,
    pub port: u16,
    pub name: String,
    pub user: String,
    pub password: String,
}

impl DbSettings {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            host: var_or("DB_HOST", "postgres"),
            port: var_or("DB_PORT", "5432").parse()?,
            name: var_or("DB_NAME", "investigation"),
            user: var_or("DB_USER", "investigator"),
            password: secret("DB_PASSWORD")?,
        })
    }

    pub fn options(&self) -> PgConnectOptions {
        PgConnectOptions::new()
            .host(&self.host)
            .port(self.port)
            .database(&self.name)
            .username(&self.user)
            .password(&self.password)
            // The database runs on the internal Compose network only.
            .ssl_mode(PgSslMode::Disable)
    }
}

/// Réglages de session appliqués à chaque connexion applicative.
pub fn tune(options: PgConnectOptions, statement_timeout: Duration) -> PgConnectOptions {
    let millis = statement_timeout.as_millis().to_string();
    options
        .options([("timezone", "UTC"), ("statement_timeout", millis.as_str()), ("lock_timeout", "5000")])
        .application_name("observatory")
}

pub async fn pool(options: PgConnectOptions, max_connections: u32) -> sqlx::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(tune(options, Duration::from_secs(15)))
        .await
}
