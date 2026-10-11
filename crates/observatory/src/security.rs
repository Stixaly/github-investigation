//! Mots de passe, sessions, CSRF et droits par investigation.
//!
//! Les formats sont ceux du socle 0.1 : hachages Argon2id PHC de pwdlib,
//! empreinte SHA-256 du jeton de session, CSRF HMAC du jeton. Les comptes et
//! les sessions ouvertes avant la migration restent donc valides.

use std::sync::LazyLock;

use anyhow::anyhow;
use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use axum::http::{HeaderMap, StatusCode};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::model::Scan;

pub const COOKIE: &str = "investigation_session";
pub const SESSION_SECONDS: i64 = 8 * 3600;
const CSRF_CONTEXT: &[u8] = b"investigation-csrf-v1";
/// Échecs tolérés en 15 minutes depuis un même client, puis tous clients confondus.
pub const MAX_FAILURES_PER_CLIENT: i64 = 5;
pub const MAX_FAILURES_PER_LOGIN: i64 = 30;

fn argon2() -> Argon2<'static> {
    // pwdlib's defaults (argon2-cffi): m=64 MiB, t=3, p=4, 32-byte hash.
    let params = Params::new(65_536, 3, 4, Some(32)).expect("valid Argon2 parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::encode_b64(&rand::random::<[u8; 16]>()).map_err(|error| anyhow!("{error}"))?;
    Ok(argon2().hash_password(password.as_bytes(), &salt).map_err(|error| anyhow!("{error}"))?.to_string())
}

/// Vérifie un mot de passe contre un hachage PHC ; les paramètres viennent du hachage.
pub fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc).is_ok_and(|hash| argon2().verify_password(password.as_bytes(), &hash).is_ok())
}

/// Hachage vérifié quand le compte n'existe pas, pour que la durée ne le révèle pas.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    hash_password("unused-dummy-password-for-timing").expect("Argon2 hashing with static parameters")
});

pub fn new_session_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn csrf_mac(token: &str) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC accepts any key length");
    mac.update(CSRF_CONTEXT);
    mac
}

pub fn csrf_token(token: &str) -> String {
    hex::encode(csrf_mac(token).finalize().into_bytes())
}

/// Comparaison en temps constant du jeton CSRF fourni par le client.
pub fn csrf_matches(token: &str, provided: &str) -> bool {
    hex::decode(provided).is_ok_and(|bytes| csrf_mac(token).verify_slice(&bytes).is_ok())
}

/// Jeton de session du cookie, s'il est présent.
pub fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.to_owned())
}

/// Refuse une requête dont l'en-tête `Origin` est présent et différent de l'origine publique.
pub fn check_origin(headers: &HeaderMap, origin: &str) -> ApiResult<()> {
    match headers.get(axum::http::header::ORIGIN) {
        Some(value) if value.as_bytes() != origin.as_bytes() => Err(ApiError::forbidden("Origine refusée")),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub login: String,
}

/// Ouvre une session. Les échecs sont comptés par compte et par client.
pub async fn authenticate(
    pool: &PgPool,
    login: &str,
    password: &str,
    client: &str,
) -> ApiResult<(User, String)> {
    let login = login.to_lowercase();
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("login:{login}"))
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM login_attempts WHERE attempted_at < now() - interval '15 minutes'")
        .execute(&mut *tx)
        .await?;
    let (from_client, overall): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE client = $2), count(*) FROM login_attempts WHERE login = $1",
    )
    .bind(&login)
    .bind(client)
    .fetch_one(&mut *tx)
    .await?;
    if from_client >= MAX_FAILURES_PER_CLIENT || overall >= MAX_FAILURES_PER_LOGIN {
        tx.commit().await?;
        return Err(ApiError::too_many("Trop de tentatives ; réessayer dans 15 minutes"));
    }
    let row: Option<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, login, password_hash FROM users WHERE login = $1")
            .bind(&login)
            .fetch_optional(&mut *tx)
            .await?;
    let hash = row.as_ref().map_or_else(|| DUMMY_HASH.clone(), |(_, _, hash)| hash.clone());
    let candidate = password.to_owned();
    let valid = tokio::task::spawn_blocking(move || verify_password(&candidate, &hash))
        .await
        .map_err(|error| anyhow!("password verification task: {error}"))?;
    let Some((id, login, _)) = row.filter(|_| valid) else {
        sqlx::query("INSERT INTO login_attempts(login, client) VALUES($1, $2)")
            .bind(&login)
            .bind(client)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(ApiError::unauthorized("Identifiants incorrects"));
    };
    sqlx::query("DELETE FROM login_attempts WHERE login = $1 AND client = $2")
        .bind(&login)
        .bind(client)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at < now()").execute(&mut *tx).await?;
    let token = new_session_token();
    sqlx::query("INSERT INTO sessions VALUES($1, $2, now() + make_interval(secs => $3::float8))")
        .bind(token_hash(&token))
        .bind(id)
        .bind(SESSION_SECONDS)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, id, "login", None).await?;
    tx.commit().await?;
    Ok((User { id, login }, token))
}

pub async fn audit(
    conn: &mut PgConnection,
    actor: Uuid,
    action: &str,
    investigation: Option<Uuid>,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO audit_events(actor_id, action, investigation_id) VALUES($1, $2, $3)")
        .bind(actor)
        .bind(action)
        .bind(investigation)
        .execute(conn)
        .await
        .map(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Owner,
}

/// Rôle de l'analyste dans l'investigation, ou 404 s'il n'en est pas membre.
///
/// Une investigation inaccessible est indiscernable d'une investigation inexistante.
pub async fn require_access(
    conn: &mut PgConnection,
    investigation_id: Uuid,
    user_id: Uuid,
    access: Access,
) -> ApiResult<String> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE investigation_id = $1 AND user_id = $2")
            .bind(investigation_id)
            .bind(user_id)
            .fetch_optional(conn)
            .await?;
    let Some(role) = role else {
        return Err(ApiError::not_found("Investigation introuvable"));
    };
    let allowed = match access {
        Access::Read => true,
        Access::Write => role != "reader",
        Access::Owner => role == "owner",
    };
    if allowed { Ok(role) } else { Err(ApiError::new(StatusCode::FORBIDDEN, "Droits insuffisants")) }
}

pub async fn accessible_scan(
    conn: &mut PgConnection,
    scan_id: Uuid,
    user_id: Uuid,
    access: Access,
) -> ApiResult<Scan> {
    let scan: Option<Scan> =
        sqlx::query_as("SELECT * FROM scans WHERE id = $1").bind(scan_id).fetch_optional(&mut *conn).await?;
    let Some(scan) = scan else {
        return Err(ApiError::not_found("Scan introuvable"));
    };
    require_access(conn, scan.investigation_id, user_id, access).await?;
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_era_hashes_still_verify() {
        // Produced by pwdlib's PasswordHash.recommended() (argon2-cffi) for "local-test-only-password-234!".
        let phc = "$argon2id$v=19$m=65536,t=3,p=4$119PsR/cW97vi6gImnW5AA$urLF7+gbxQMk9jXY+6sCBPWW1GjGTPh4+w7rkblkp+g";
        assert!(verify_password("local-test-only-password-234!", phc));
        assert!(!verify_password("wrong", phc));
    }

    #[test]
    fn new_hashes_round_trip() {
        let phc = hash_password("correct horse battery staple").unwrap();
        assert!(phc.starts_with("$argon2id$v=19$m=65536,t=3,p=4$"));
        assert!(verify_password("correct horse battery staple", &phc));
        assert!(!verify_password("correct horse battery stapler", &phc));
        assert!(!verify_password("x", "not a phc string"));
    }

    #[test]
    fn csrf_is_bound_to_the_session() {
        let token = new_session_token();
        assert_eq!(token.len(), 43);
        let csrf = csrf_token(&token);
        assert!(csrf_matches(&token, &csrf));
        assert!(!csrf_matches(&new_session_token(), &csrf));
        assert!(!csrf_matches(&token, ""));
        assert!(!csrf_matches(&token, "zz"));
    }

    #[test]
    fn session_cookie_is_found_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", "theme=dark; investigation_session=abc=; other=1".parse().unwrap());
        assert_eq!(session_cookie(&headers).as_deref(), Some("abc="));
    }
}
