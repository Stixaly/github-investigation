//! Enregistrement des observations GitHub : preuves, entités, relations.
//!
//! Règle centrale : un objet privé ou non public n'est jamais persisté, pas
//! même comme preuve. Les entités et relations sont partagées entre
//! investigations ; leur observation est rattachée à un scan.

use chrono::{DateTime, Utc};
use observatory_domain::evidence_hash;
use serde_json::{Map, Value};
use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    Instant,
    Day,
    Unknown,
}

impl Precision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Instant => "instant",
            Self::Day => "day",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Account,
    Repository,
    Issue,
    PullRequest,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Account => "Account",
            Self::Repository => "Repository",
            Self::Issue => "Issue",
            Self::PullRequest => "PullRequest",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    Owns,
    ForkOf,
    ContributedTo,
    Authored,
    InRepository,
    Starred,
}

impl RelationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owns => "OWNS",
            Self::ForkOf => "FORK_OF",
            Self::ContributedTo => "CONTRIBUTED_TO",
            Self::Authored => "AUTHORED",
            Self::InRepository => "IN_REPOSITORY",
            Self::Starred => "STARRED",
        }
    }
}

/// Rôle d'un objet dans la réponse GitHub qui l'a fourni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Repository,
    Account,
    Contributors,
    Forks,
    Issues,
    Events,
    Repositories,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Account => "account",
            Self::Contributors => "contributors",
            Self::Forks => "forks",
            Self::Issues => "issues",
            Self::Events => "events",
            Self::Repositories => "repositories",
        }
    }

    fn lists_repositories(self) -> bool {
        matches!(self, Self::Repository | Self::Forks | Self::Repositories)
    }
}

/// Contexte d'une observation : scan, preuve et date.
#[derive(Debug, Clone, Copy)]
pub struct Observation<'a> {
    pub scan_id: Uuid,
    pub proof: &'a str,
    pub observed: Option<DateTime<Utc>>,
    pub precision: Precision,
}

/// Valeur « vraie » au sens de Python : ni null, ni false, ni 0, ni vide.
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

/// Un objet est hors périmètre s'il est privé ou si sa visibilité n'est pas `public`.
pub fn is_private(item: &Value) -> bool {
    truthy(item.get("private")) || item.get("visibility").is_some_and(|v| v.as_str() != Some("public"))
}

/// Identifiant GitHub stable sous forme de texte : nombre ou chaîne.
pub fn external_id(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

fn text<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str).filter(|value| !value.is_empty())
}

fn timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?.as_str()?).ok().map(|at| at.with_timezone(&Utc))
}

/// Enregistre la preuve une seule fois, identifiée par son empreinte.
pub async fn evidence(conn: &mut PgConnection, payload: &Value, source: &str) -> sqlx::Result<String> {
    let digest = evidence_hash(source, payload);
    sqlx::query("INSERT INTO evidence(hash, payload, source) VALUES($1, $2, $3) ON CONFLICT DO NOTHING")
        .bind(&digest)
        .bind(payload)
        .bind(source)
        .execute(conn)
        .await?;
    Ok(digest)
}

/// Entité canonique et son observation. `data` vaut `None` quand l'objet observé
/// est la preuve elle-même : il n'est alors pas recopié (migration 0002).
pub async fn entity(
    conn: &mut PgConnection,
    observation: &Observation<'_>,
    kind: EntityKind,
    external_id: &str,
    label: &str,
    data: Option<&Value>,
    provisional: bool,
) -> sqlx::Result<Uuid> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO entities(id, kind, external_id, label, provisional) VALUES($1, $2, $3, $4, $5) \
         ON CONFLICT(kind, external_id) DO UPDATE SET label = excluded.label RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(kind.as_str())
    .bind(external_id)
    .bind(label)
    .bind(provisional)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO scan_entities(scan_id, entity_id, evidence_hash, data, observed_at, time_precision) \
         VALUES($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
    )
    .bind(observation.scan_id)
    .bind(id)
    .bind(observation.proof)
    .bind(data)
    .bind(observation.observed)
    .bind(observation.precision.as_str())
    .execute(conn)
    .await?;
    Ok(id)
}

pub async fn relation(
    conn: &mut PgConnection,
    observation: &Observation<'_>,
    source: Uuid,
    kind: RelationKind,
    target: Uuid,
    event_at: Option<DateTime<Utc>>,
) -> sqlx::Result<()> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO relations(id, source_id, kind, target_id, context) VALUES($1, $2, $3, $4, '') \
         ON CONFLICT(source_id, kind, target_id, context) DO UPDATE SET context = excluded.context RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(source)
    .bind(kind.as_str())
    .bind(target)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO scan_relations(scan_id, relation_id, evidence_hash, observed_at, source_event_at, \
         time_precision) VALUES($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
    )
    .bind(observation.scan_id)
    .bind(id)
    .bind(observation.proof)
    .bind(observation.observed)
    .bind(event_at)
    .bind(observation.precision.as_str())
    .execute(conn)
    .await?;
    Ok(())
}

/// Compte GitHub, s'il a un identifiant et un login. `is_proof` : `data` est la preuve entière.
pub async fn account(
    conn: &mut PgConnection,
    observation: &Observation<'_>,
    data: Option<&Value>,
    is_proof: bool,
) -> sqlx::Result<Option<Uuid>> {
    let Some(map) = data.and_then(Value::as_object).filter(|map| !map.is_empty()) else {
        return Ok(None);
    };
    let (Some(id), Some(login)) = (external_id(map.get("id")), text(map, "login")) else {
        return Ok(None);
    };
    let copy = if is_proof { None } else { data };
    entity(conn, observation, EntityKind::Account, &id, login, copy, false).await.map(Some)
}

/// Dépôt public et son propriétaire.
pub async fn repository(
    conn: &mut PgConnection,
    observation: &Observation<'_>,
    data: Option<&Value>,
    is_proof: bool,
) -> sqlx::Result<Option<Uuid>> {
    let Some(item) = data.filter(|item| item.is_object() && !is_private(item)) else {
        return Ok(None);
    };
    let map = item.as_object().expect("checked above");
    let (Some(id), Some(full_name)) = (external_id(map.get("id")), text(map, "full_name")) else {
        return Ok(None);
    };
    let copy = if is_proof { None } else { data };
    let target = entity(conn, observation, EntityKind::Repository, &id, full_name, copy, false).await?;
    if let Some(owner) = account(conn, observation, map.get("owner"), false).await? {
        relation(conn, observation, owner, RelationKind::Owns, target, None).await?;
    }
    Ok(Some(target))
}

/// Enregistre un objet renvoyé par l'API GitHub selon son rôle dans la collecte.
///
/// Retourne l'entité principale de l'objet, `None` s'il est hors périmètre ou incomplet.
pub async fn ingest_github(
    conn: &mut PgConnection,
    scan_id: Uuid,
    role: Role,
    item: &Value,
    source_url: &str,
    observed: DateTime<Utc>,
    root: Option<Uuid>,
) -> sqlx::Result<Option<Uuid>> {
    if role == Role::Events && item.get("public") != Some(&Value::Bool(true)) {
        return Ok(None);
    }
    if role.lists_repositories() && is_private(item) {
        return Ok(None);
    }
    let proof = evidence(conn, item, source_url).await?;
    let observation =
        Observation { scan_id, proof: &proof, observed: Some(observed), precision: Precision::Instant };
    match role {
        Role::Repository | Role::Forks | Role::Repositories => {
            let Some(target) = repository(conn, &observation, Some(item), true).await? else {
                return Ok(None);
            };
            if role == Role::Forks
                && let Some(root) = root
            {
                relation(conn, &observation, target, RelationKind::ForkOf, root, None).await?;
            }
            if let Some(parent) = repository(conn, &observation, item.get("parent"), false).await? {
                relation(conn, &observation, target, RelationKind::ForkOf, parent, None).await?;
            }
            Ok(Some(target))
        }
        Role::Account | Role::Contributors => {
            let actor = account(conn, &observation, Some(item), true).await?;
            if role == Role::Contributors
                && let (Some(actor), Some(root)) = (actor, root)
            {
                relation(conn, &observation, actor, RelationKind::ContributedTo, root, None).await?;
            }
            Ok(actor)
        }
        Role::Issues => {
            let (Some(root), Some(id)) = (root, external_id(item.get("id"))) else {
                return Ok(None);
            };
            let kind =
                if truthy(item.get("pull_request")) { EntityKind::PullRequest } else { EntityKind::Issue };
            let number = item.get("number").map_or_else(|| "?".to_owned(), Value::to_string);
            let title = item.get("title").and_then(Value::as_str).unwrap_or_default();
            let label = format!("#{number} {title}");
            let target = entity(conn, &observation, kind, &id, &label, None, false).await?;
            if let Some(actor) = account(conn, &observation, item.get("user"), false).await? {
                let created = timestamp(item.get("created_at"));
                relation(conn, &observation, actor, RelationKind::Authored, target, created).await?;
            }
            relation(conn, &observation, target, RelationKind::InRepository, root, None).await?;
            Ok(Some(target))
        }
        Role::Events => {
            let Some(root) = root else {
                return Ok(None);
            };
            let actor = account(conn, &observation, item.get("actor"), false).await?;
            let Some(actor) = actor else {
                return Ok(None);
            };
            match item.get("type").and_then(Value::as_str) {
                Some("WatchEvent") => {
                    let created = timestamp(item.get("created_at"));
                    relation(conn, &observation, actor, RelationKind::Starred, root, created).await?;
                }
                Some("ForkEvent") => {
                    let forkee = item.get("payload").and_then(|payload| payload.get("forkee"));
                    if let Some(fork) = repository(conn, &observation, forkee, false).await? {
                        relation(conn, &observation, fork, RelationKind::ForkOf, root, None).await?;
                    }
                }
                _ => {}
            }
            Ok(Some(actor))
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn python_truthiness() {
        for value in [json!(null), json!(false), json!(0), json!(0.0), json!(""), json!([]), json!({})] {
            assert!(!truthy(Some(&value)), "{value}");
        }
        for value in [json!(true), json!(1), json!("x"), json!([0]), json!({"a": 1})] {
            assert!(truthy(Some(&value)), "{value}");
        }
        assert!(!truthy(None));
    }

    #[test]
    fn privacy_rules() {
        assert!(!is_private(&json!({"private": false})));
        assert!(!is_private(&json!({"visibility": "public"})));
        assert!(is_private(&json!({"private": true})));
        assert!(is_private(&json!({"visibility": "internal"})));
        assert!(is_private(&json!({"visibility": null})));
    }

    #[test]
    fn identifiers_are_numbers_or_strings() {
        assert_eq!(external_id(Some(&json!(123))), Some("123".into()));
        assert_eq!(external_id(Some(&json!("demo:account:1"))), Some("demo:account:1".into()));
        assert_eq!(external_id(Some(&json!(null))), None);
        assert_eq!(external_id(Some(&json!(true))), None);
    }
}
