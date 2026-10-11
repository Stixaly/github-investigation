//! Migrations numérotées, vérifiées par empreinte.
//!
//! Chaque fichier `migrations/NNNN_nom.sql` est appliqué une seule fois, dans
//! sa propre transaction, et son SHA-256 est enregistré. Une migration modifiée
//! après application, ou une base plus récente que le binaire, bloque le
//! démarrage. La table `schema_migrations` est celle du socle 0.1 : la
//! migration 0001 est le `schema.sql` d'origine, à l'octet près.

use observatory_domain::canonical::sha256_hex;
use sqlx::{Connection, PgConnection, PgPool};

/// Rôle PostgreSQL de l'API et du worker : lecture et écriture des données, aucun DDL.
pub const APP_ROLE: &str = "investigator";
const LOCK_KEY: i64 = 482_760;

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: i32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, name: "initial", sql: include_str!("../../../migrations/0001_initial.sql") },
    Migration { version: 2, name: "hardening", sql: include_str!("../../../migrations/0002_hardening.sql") },
    Migration {
        version: 3,
        name: "collecte_econome",
        sql: include_str!("../../../migrations/0003_collecte_econome.sql"),
    },
    Migration {
        version: 4,
        name: "graphe_dossier",
        sql: include_str!("../../../migrations/0004_graphe_dossier.sql"),
    },
];

pub fn latest() -> i32 {
    MIGRATIONS.last().map_or(0, |migration| migration.version)
}

/// Empreinte d'une migration, indépendante des fins de ligne du checkout.
pub fn checksum(sql: &str) -> String {
    sha256_hex(&sql.replace("\r\n", "\n"))
}

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("Historique de migration incompatible : la migration {0} a été modifiée après application")]
    Altered(i32),
    #[error("La base contient la migration {0}, inconnue de ce binaire : mettre à jour l'application")]
    Unknown(i32),
    #[error("Schéma en version {current}, version {expected} attendue : exécuter `observatory migrate`")]
    Behind { current: i32, expected: i32 },
    #[error("Mot de passe du rôle applicatif absent ou invalide")]
    AppPassword,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Compare l'historique appliqué aux migrations connues ; retourne celles qui restent à appliquer.
pub fn pending<'a>(
    applied: &[(i32, String)],
    known: &'a [Migration],
) -> Result<Vec<&'a Migration>, SchemaError> {
    for (version, recorded) in applied {
        let Some(migration) = known.iter().find(|migration| migration.version == *version) else {
            return Err(SchemaError::Unknown(*version));
        };
        if *recorded != checksum(migration.sql) {
            return Err(SchemaError::Altered(*version));
        }
    }
    Ok(known
        .iter()
        .filter(|migration| applied.iter().all(|(version, _)| *version != migration.version))
        .collect())
}

async fn applied(conn: &mut PgConnection) -> sqlx::Result<Vec<(i32, String)>> {
    let exists: bool = sqlx::query_scalar("SELECT to_regclass('schema_migrations') IS NOT NULL")
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(Vec::new());
    }
    sqlx::query_as("SELECT version, checksum FROM schema_migrations ORDER BY version").fetch_all(conn).await
}

/// Vérification faite au démarrage de l'API et du worker : schéma exactement à jour.
pub async fn verify(pool: &PgPool) -> Result<(), SchemaError> {
    let mut conn = pool.acquire().await?;
    let applied = applied(&mut conn).await?;
    let missing = pending(&applied, MIGRATIONS)?;
    if let Some(first) = missing.first() {
        let current = applied.last().map_or(0, |(version, _)| *version);
        return Err(SchemaError::Behind { current: current.min(first.version - 1), expected: latest() });
    }
    Ok(())
}

/// Applique les migrations jusqu'à `target` (toutes par défaut), puis les droits du rôle applicatif.
///
/// `conn` doit appartenir au propriétaire du schéma. Retourne les versions appliquées.
pub async fn migrate(
    conn: &mut PgConnection,
    app_password: Option<&str>,
    target: Option<i32>,
) -> Result<Vec<i32>, SchemaError> {
    sqlx::query("SELECT pg_advisory_lock($1)").bind(LOCK_KEY).execute(&mut *conn).await?;
    let result = migrate_locked(conn, app_password, target).await;
    sqlx::query("SELECT pg_advisory_unlock($1)").bind(LOCK_KEY).execute(&mut *conn).await?;
    result
}

async fn migrate_locked(
    conn: &mut PgConnection,
    app_password: Option<&str>,
    target: Option<i32>,
) -> Result<Vec<i32>, SchemaError> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version integer PRIMARY KEY, checksum text NOT NULL); \
         ALTER TABLE schema_migrations ADD COLUMN IF NOT EXISTS applied_at timestamptz",
    )
    .execute(&mut *conn)
    .await?;
    let applied = applied(conn).await?;
    let mut done = Vec::new();
    for migration in pending(&applied, MIGRATIONS)? {
        if target.is_some_and(|target| migration.version > target) {
            break;
        }
        let mut tx = conn.begin().await?;
        sqlx::raw_sql(migration.sql).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO schema_migrations(version, checksum, applied_at) VALUES($1, $2, now())")
            .bind(migration.version)
            .bind(checksum(migration.sql))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::info!(version = migration.version, name = migration.name, "migration applied");
        done.push(migration.version);
    }
    if let Some(password) = app_password {
        grant_app_role(conn, password).await?;
    }
    Ok(done)
}

async fn grant_app_role(conn: &mut PgConnection, password: &str) -> Result<(), SchemaError> {
    if password.is_empty() || password.contains('\0') {
        return Err(SchemaError::AppPassword);
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)")
        .bind(APP_ROLE)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        // Utility statements take no bind parameters: quote the literal, which is
        // only safe with standard_conforming_strings (the default since PostgreSQL 9.1).
        let standard: String =
            sqlx::query_scalar("SHOW standard_conforming_strings").fetch_one(&mut *conn).await?;
        if standard != "on" {
            return Err(SchemaError::AppPassword);
        }
        let literal = format!("'{}'", password.replace('\'', "''"));
        // APP_ROLE is a constant and `literal` the quoted password checked above.
        let created =
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE ROLE {APP_ROLE} LOGIN PASSWORD {literal}")))
                .execute(&mut *conn)
                .await;
        match created {
            // Another migrator created it concurrently (roles are cluster-wide).
            Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("42710") => {}
            other => {
                other?;
            }
        }
    }
    // Only the constant APP_ROLE is interpolated.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE CREATE ON SCHEMA public FROM PUBLIC; \
         GRANT USAGE ON SCHEMA public TO {APP_ROLE}; \
         GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO {APP_ROLE}; \
         REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON schema_migrations FROM {APP_ROLE}; \
         GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO {APP_ROLE}"
    )))
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_migration_is_the_python_schema() {
        // sha256 recorded by the 0.1 migrator: hashlib.sha256(schema.sql.read_text().encode()).
        assert_eq!(
            checksum(MIGRATIONS[0].sql),
            "24b37b0857e139ff94d288f977a4f9e02c313530da30fde2424137d913fab7f0"
        );
        assert_eq!(checksum("a\r\nb"), checksum("a\nb"));
    }

    #[test]
    fn versions_are_contiguous() {
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(usize::try_from(migration.version).unwrap(), index + 1);
        }
    }

    #[test]
    fn history_is_checked() {
        let v1 = (1, checksum(MIGRATIONS[0].sql));
        assert_eq!(pending(&[], MIGRATIONS).unwrap().len(), MIGRATIONS.len());
        assert_eq!(pending(std::slice::from_ref(&v1), MIGRATIONS).unwrap()[0].version, 2);
        assert!(matches!(pending(&[(1, "edited".into())], MIGRATIONS), Err(SchemaError::Altered(1))));
        assert!(matches!(pending(&[v1, (99, String::new())], MIGRATIONS), Err(SchemaError::Unknown(99))));
    }
}
