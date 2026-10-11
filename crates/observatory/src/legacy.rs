//! Import des anciens instantanés `users_db.json` et `repos_db.json` de ctimap-tool.
//!
//! Seuls ces deux fichiers JSON inertes sont lus, en flux : un dépôt à la fois,
//! sans charger la liste complète d'un compte. `sets.pkl` et `token_key.txt` ne
//! sont jamais ouverts. L'import est idempotent : relancé sur les mêmes fichiers
//! et la même date, il reprend ou réutilise le scan existant.

use std::collections::HashMap;

use std::fs::File;
use std::io::{BufReader, Read};

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use chrono::{NaiveDate, TimeZone, Utc};
use observatory_domain::canonical::{PYTHON_DEFAULT, sha256_hex, to_string};
use observatory_domain::snapshot::{for_each_repository, for_each_user};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::security::{Access, require_access};
use crate::store::{Batch, EntityKind, EntityRef, Observation, Precision, RelationKind, truthy};

/// Lignes préparées puis écrites ensemble, dans une transaction.
const BATCH: usize = 1000;

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Counters {
    pub users: u64,
    pub repositories: u64,
    pub user_errors: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ImportReport {
    pub scan_id: Uuid,
    pub reused: bool,
    #[serde(flatten)]
    pub counters: Counters,
}

/// SHA-256 d'un fichier, lu par blocs.
pub fn file_hash(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(hex::encode(hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
    }
}

/// Clé d'idempotence de l'import : celle du socle 0.1, qui hachait
/// `json.dumps(manifest, sort_keys=True)` avec les séparateurs par défaut.
pub fn import_key(manifest: &Value) -> String {
    sha256_hex(&to_string(manifest, PYTHON_DEFAULT))
}

/// Lit un fichier JSON dans un thread bloquant et transmet les entrées au fil des lectures.
fn stream<T: Send + 'static>(
    path: PathBuf,
    parse: impl FnOnce(BufReader<File>, &mut dyn FnMut(T) -> Result<(), String>) -> serde_json::Result<()>
    + Send
    + 'static,
) -> (mpsc::Receiver<T>, tokio::task::JoinHandle<anyhow::Result<()>>) {
    let (sender, receiver) = mpsc::channel(256);
    let task = tokio::task::spawn_blocking(move || {
        let reader = BufReader::new(File::open(&path).with_context(|| path.display().to_string())?);
        let mut send = |entry: T| sender.blocking_send(entry).map_err(|_| "import interrompu".to_owned());
        parse(reader, &mut send).with_context(|| format!("lecture de {}", path.display()))
    });
    (receiver, task)
}

#[derive(Debug, Clone)]
pub struct ImportRequest {
    pub directory: PathBuf,
    pub investigation: Uuid,
    pub login: String,
    pub observed_date: Option<NaiveDate>,
}

pub async fn import_snapshot(pool: &PgPool, request: &ImportRequest) -> anyhow::Result<ImportReport> {
    let directory = request.directory.canonicalize().context("dossier de l'instantané")?;
    let users_path = directory.join("users_db.json");
    let repos_path = directory.join("repos_db.json");
    let (users_digest, repos_digest) = {
        let (users, repos) = (users_path.clone(), repos_path.clone());
        tokio::task::spawn_blocking(move || Ok::<_, std::io::Error>((file_hash(&users)?, file_hash(&repos)?)))
            .await??
    };
    let observed_date = request.observed_date.map(|date| date.format("%Y-%m-%d").to_string());
    let manifest = json!({
        "files": {"users": users_digest, "repos": repos_digest},
        "observed_date": observed_date,
        "version": 1,
    });
    let key = import_key(&manifest);
    let observed =
        request.observed_date.map(|date| Utc.from_utc_datetime(&date.and_time(chrono::NaiveTime::MIN)));
    let precision = if observed.is_some() { Precision::Day } else { Precision::Unknown };

    let mut conn = pool.acquire().await?;
    let user: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE login = $1")
        .bind(request.login.to_lowercase())
        .fetch_optional(&mut *conn)
        .await?;
    let Some(user) = user else { bail!("Compte analyste introuvable") };
    require_access(&mut conn, request.investigation, user, Access::Write)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let existing: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, status FROM scans WHERE investigation_id = $1 AND import_key = $2")
            .bind(request.investigation)
            .bind(&key)
            .fetch_optional(&mut *conn)
            .await?;
    let scan_id = match existing {
        Some((id, status)) if status == "succeeded" => {
            return Ok(ImportReport { scan_id: id, reused: true, counters: Counters::default() });
        }
        Some((id, _)) => id,
        None => {
            let id = Uuid::new_v4();
            let seed =
                directory.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
            sqlx::query(
                "INSERT INTO scans(id, investigation_id, requested_by, source, seed, status, import_key, \
                 idempotency_key, started_at) VALUES($1, $2, $3, 'legacy', $4, 'running', $5, $6, now())",
            )
            .bind(id)
            .bind(request.investigation)
            .bind(user)
            .bind(seed)
            .bind(&key)
            .bind(Uuid::new_v4())
            .execute(&mut *conn)
            .await?;
            id
        }
    };
    drop(conn);

    let mut counters = Counters::default();
    let outcome = load(
        pool,
        scan_id,
        &users_path,
        &repos_path,
        (&users_digest, &repos_digest),
        observed,
        precision,
        &mut counters,
    )
    .await;
    match outcome {
        Ok(()) => {
            let coverage = json!({
                "import": {"status": "complete", "users": counters.users, "repositories": counters.repositories,
                           "user_errors": counters.user_errors, "skipped": counters.skipped, "manifest": manifest},
                "roles": {"status": "not_imported", "reason": "Pickle non chargé ; relations historiques non reconstruites"},
            });
            sqlx::query("UPDATE scans SET status = 'succeeded', finished_at = now(), coverage = $1, error = NULL WHERE id = $2")
                .bind(coverage)
                .bind(scan_id)
                .execute(pool)
                .await?;
            Ok(ImportReport { scan_id, reused: false, counters })
        }
        Err(error) => {
            sqlx::query("UPDATE scans SET status = 'failed', error = 'Import interrompu ; relancer le même import' WHERE id = $1")
                .bind(scan_id)
                .execute(pool)
                .await?;
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn load(
    pool: &PgPool,
    scan_id: Uuid,
    users_path: &Path,
    repos_path: &Path,
    (users_digest, repos_digest): (&str, &str),
    observed: Option<chrono::DateTime<Utc>>,
    precision: Precision,
    counters: &mut Counters,
) -> anyhow::Result<()> {
    let mut identities: HashMap<String, EntityRef> = HashMap::new();

    let (mut users, reader) = stream::<(String, Value)>(users_path.to_owned(), |file, send| {
        for_each_user(file, |name, user| send((name, user)))
    });
    let mut batch = Batch::default();
    let mut pending: usize = 0;
    while let Some((name, user)) = users.recv().await {
        if !user.is_object() || user.get("error").is_some() {
            counters.user_errors += 1;
            continue;
        }
        let proof = batch.evidence(&user, &format!("legacy:sha256:{users_digest}:users_db.json:{name}"));
        let observation = Observation { scan_id, proof: &proof, observed, precision };
        match batch.account(&observation, Some(&user), true) {
            Some(identity) => {
                identities.insert(name.to_lowercase(), identity);
                counters.users += 1;
            }
            None => counters.skipped += 1,
        }
        pending += 1;
        if pending.is_multiple_of(BATCH) {
            flush(pool, std::mem::take(&mut batch)).await?;
        }
    }
    flush(pool, batch).await?;
    reader.await??;

    let (mut repositories, reader) = stream::<(String, Value)>(repos_path.to_owned(), |file, send| {
        for_each_repository(file, |owner, repository| send((owner.to_owned(), repository)))
    });
    let mut batch = Batch::default();
    while let Some((owner_name, repository)) = repositories.recv().await {
        let full_name = repository.get("full_name").and_then(Value::as_str).filter(|name| !name.is_empty());
        let Some(full_name) = full_name.filter(|_| !truthy(repository.get("private"))) else {
            counters.skipped += 1;
            continue;
        };
        let proof =
            batch.evidence(&repository, &format!("legacy:sha256:{repos_digest}:repos_db.json:{owner_name}"));
        let observation = Observation { scan_id, proof: &proof, observed, precision };
        let stable = crate::store::external_id(repository.get("id"));
        let provisional = stable.is_none();
        let external =
            stable.unwrap_or_else(|| format!("legacy:{repos_digest}:{}", full_name.to_lowercase()));
        let target =
            batch.entity(&observation, EntityKind::Repository, &external, full_name, None, provisional);
        // Link the owner only when the repository name agrees with the bucket it was stored under.
        let owner = identities.get(&owner_name.to_lowercase());
        let prefix = full_name.split('/').next().unwrap_or_default();
        if let Some(owner) = owner.filter(|_| prefix.to_lowercase() == owner_name.to_lowercase()) {
            batch.relation(&observation, owner.clone(), RelationKind::Owns, target, None);
        }
        counters.repositories += 1;
        if counters.repositories.is_multiple_of(BATCH as u64) {
            flush(pool, std::mem::take(&mut batch)).await?;
        }
    }
    flush(pool, batch).await?;
    reader.await??;
    Ok(())
}

/// Écrit un lot dans sa propre transaction.
async fn flush(pool: &PgPool, batch: Batch) -> sqlx::Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    batch.write(&mut tx).await?;
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_key_matches_python() {
        // hashlib.sha256(json.dumps({"files": {"users": "ab", "repos": "cd"}, "observed_date": None,
        //   "version": 1}, sort_keys=True).encode()).hexdigest()
        let manifest = json!({"files": {"users": "ab", "repos": "cd"}, "observed_date": null, "version": 1});
        assert_eq!(import_key(&manifest), "e19efc079463fec5f4e996c74c895573d41a72ae0f6219a3875df9d3fed3be3b");
    }
}
