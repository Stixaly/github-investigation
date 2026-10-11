//! Enregistrement des observations GitHub : preuves, entités, relations.
//!
//! Règle centrale : un objet privé ou non public n'est jamais persisté, pas
//! même comme preuve. Les entités et relations sont partagées entre
//! investigations ; leur observation est rattachée à un scan.
//!
//! Les écritures passent par un [`Batch`] : les objets d'une page (ou d'un
//! paquet d'import) sont préparés en mémoire, puis écrits en cinq requêtes
//! ensemblistes au plus, au lieu de plusieurs requêtes par objet. Les
//! empreintes de preuve et l'idempotence ne changent pas.

use std::collections::{HashMap, HashSet};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

    fn parse(value: &str) -> Option<Self> {
        [Self::Account, Self::Repository, Self::Issue, Self::PullRequest]
            .into_iter()
            .find(|kind| kind.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// Entité désignée dans un lot : déjà enregistrée, ou identifiée par son type et son identifiant externe.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EntityRef {
    Id(Uuid),
    Key(EntityKind, String),
}

#[derive(Debug)]
struct NewEntity {
    kind: EntityKind,
    external_id: String,
    label: String,
    provisional: bool,
}

#[derive(Debug)]
struct EntityObservation {
    scan_id: Uuid,
    entity: (EntityKind, String),
    evidence: String,
    data: Option<Value>,
    observed: Option<DateTime<Utc>>,
    precision: Precision,
}

#[derive(Debug)]
struct RelationObservation {
    scan_id: Uuid,
    source: EntityRef,
    kind: RelationKind,
    target: EntityRef,
    evidence: String,
    observed: Option<DateTime<Utc>>,
    event_at: Option<DateTime<Utc>>,
    precision: Precision,
}

/// Preuve d'un objet préparé : l'objet lui-même, ou la réponse reçue dont il est tiré.
#[derive(Debug, Clone, Copy)]
enum Proof<'a> {
    Itself(&'a str),
    Received(&'a Value, &'a str),
}

/// Identifiants attribués par l'écriture d'un lot.
#[derive(Debug, Default)]
pub struct Written {
    ids: HashMap<(EntityKind, String), Uuid>,
}

impl Written {
    pub fn id(&self, entity: &EntityRef) -> Option<Uuid> {
        match entity {
            EntityRef::Id(id) => Some(*id),
            EntityRef::Key(kind, external_id) => self.ids.get(&(*kind, external_id.clone())).copied(),
        }
    }
}

/// Écritures préparées en mémoire, puis envoyées ensemble par [`Batch::write`].
///
/// Le résultat est celui des écritures successives qu'il remplace : une
/// entité vue deux fois garde le dernier libellé et son premier état
/// provisoire ; la première observation d'un même triplet scan, objet et
/// preuve l'emporte.
#[derive(Debug, Default)]
pub struct Batch {
    evidence: Vec<(String, Value, String)>,
    known_evidence: HashSet<String>,
    entities: Vec<NewEntity>,
    entity_index: HashMap<(EntityKind, String), usize>,
    entity_observations: Vec<EntityObservation>,
    relation_observations: Vec<RelationObservation>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.evidence.is_empty()
            && self.entity_observations.is_empty()
            && self.relation_observations.is_empty()
    }

    /// Prépare la preuve, identifiée par son empreinte ; retourne l'empreinte.
    pub fn evidence(&mut self, payload: &Value, source: &str) -> String {
        let digest = evidence_hash(source, payload);
        if self.known_evidence.insert(digest.clone()) {
            self.evidence.push((digest.clone(), payload.clone(), source.to_owned()));
        }
        digest
    }

    /// Entité canonique et son observation. `data` vaut `None` quand l'objet observé
    /// est la preuve elle-même : il n'est alors pas recopié (migration 0002).
    pub fn entity(
        &mut self,
        observation: &Observation<'_>,
        kind: EntityKind,
        external_id: &str,
        label: &str,
        data: Option<&Value>,
        provisional: bool,
    ) -> EntityRef {
        let key = (kind, external_id.to_owned());
        if let Some(&index) = self.entity_index.get(&key) {
            label.clone_into(&mut self.entities[index].label);
        } else {
            self.entity_index.insert(key.clone(), self.entities.len());
            self.entities.push(NewEntity {
                kind,
                external_id: external_id.to_owned(),
                label: label.to_owned(),
                provisional,
            });
        }
        self.entity_observations.push(EntityObservation {
            scan_id: observation.scan_id,
            entity: key,
            evidence: observation.proof.to_owned(),
            data: data.cloned(),
            observed: observation.observed,
            precision: observation.precision,
        });
        EntityRef::Key(kind, external_id.to_owned())
    }

    pub fn relation(
        &mut self,
        observation: &Observation<'_>,
        source: EntityRef,
        kind: RelationKind,
        target: EntityRef,
        event_at: Option<DateTime<Utc>>,
    ) {
        self.relation_observations.push(RelationObservation {
            scan_id: observation.scan_id,
            source,
            kind,
            target,
            evidence: observation.proof.to_owned(),
            observed: observation.observed,
            event_at,
            precision: observation.precision,
        });
    }

    /// Compte GitHub, s'il a un identifiant et un login. `is_proof` : `data` est la preuve entière.
    pub fn account(
        &mut self,
        observation: &Observation<'_>,
        data: Option<&Value>,
        is_proof: bool,
    ) -> Option<EntityRef> {
        let map = data.and_then(Value::as_object).filter(|map| !map.is_empty())?;
        let (Some(id), Some(login)) = (external_id(map.get("id")), text(map, "login")) else {
            return None;
        };
        let copy = if is_proof { None } else { data };
        Some(self.entity(observation, EntityKind::Account, &id, login, copy, false))
    }

    /// Dépôt public et son propriétaire.
    pub fn repository(
        &mut self,
        observation: &Observation<'_>,
        data: Option<&Value>,
        is_proof: bool,
    ) -> Option<EntityRef> {
        let item = data.filter(|item| item.is_object() && !is_private(item))?;
        let map = item.as_object()?;
        let (Some(id), Some(full_name)) = (external_id(map.get("id")), text(map, "full_name")) else {
            return None;
        };
        let copy = if is_proof { None } else { data };
        let target = self.entity(observation, EntityKind::Repository, &id, full_name, copy, false);
        if let Some(owner) = self.account(observation, map.get("owner"), false) {
            self.relation(observation, owner, RelationKind::Owns, target.clone(), None);
        }
        Some(target)
    }

    /// Prépare un objet renvoyé par l'API GitHub selon son rôle dans la collecte.
    ///
    /// Retourne l'entité principale de l'objet, `None` s'il est hors périmètre ou incomplet.
    pub fn ingest_github(
        &mut self,
        scan_id: Uuid,
        role: Role,
        item: &Value,
        source_url: &str,
        observed: DateTime<Utc>,
        root: Option<&EntityRef>,
    ) -> Option<EntityRef> {
        self.ingest(scan_id, role, item, Proof::Itself(source_url), observed, root)
    }

    /// Comme [`Batch::ingest_github`], pour un objet mis en forme REST à partir d'une
    /// réponse reçue sous une autre forme (nœud GraphQL) : la réponse reste la preuve,
    /// telle que reçue, et l'objet est recopié dans l'observation.
    pub fn ingest_received(
        &mut self,
        scan_id: Uuid,
        role: Role,
        item: &Value,
        (received, source): (&Value, &str),
        observed: DateTime<Utc>,
        root: Option<&EntityRef>,
    ) -> Option<EntityRef> {
        self.ingest(scan_id, role, item, Proof::Received(received, source), observed, root)
    }

    fn ingest(
        &mut self,
        scan_id: Uuid,
        role: Role,
        item: &Value,
        proof: Proof<'_>,
        observed: DateTime<Utc>,
        root: Option<&EntityRef>,
    ) -> Option<EntityRef> {
        if role == Role::Events && item.get("public") != Some(&Value::Bool(true)) {
            return None;
        }
        if role.lists_repositories() && is_private(item) {
            return None;
        }
        let (payload, source, is_proof) = match proof {
            Proof::Itself(source) => (item, source, true),
            Proof::Received(received, source) => (received, source, false),
        };
        let proof = self.evidence(payload, source);
        let observation =
            Observation { scan_id, proof: &proof, observed: Some(observed), precision: Precision::Instant };
        match role {
            Role::Repository | Role::Forks | Role::Repositories => {
                let target = self.repository(&observation, Some(item), is_proof)?;
                if role == Role::Forks
                    && let Some(root) = root
                {
                    self.relation(&observation, target.clone(), RelationKind::ForkOf, root.clone(), None);
                }
                if let Some(parent) = self.repository(&observation, item.get("parent"), false) {
                    self.relation(&observation, target.clone(), RelationKind::ForkOf, parent, None);
                }
                Some(target)
            }
            Role::Account | Role::Contributors => {
                let actor = self.account(&observation, Some(item), is_proof)?;
                if role == Role::Contributors
                    && let Some(root) = root
                {
                    self.relation(
                        &observation,
                        actor.clone(),
                        RelationKind::ContributedTo,
                        root.clone(),
                        None,
                    );
                }
                Some(actor)
            }
            Role::Issues => {
                let (root, id) = (root?, external_id(item.get("id"))?);
                let kind = if truthy(item.get("pull_request")) {
                    EntityKind::PullRequest
                } else {
                    EntityKind::Issue
                };
                let number = item.get("number").map_or_else(|| "?".to_owned(), Value::to_string);
                let title = item.get("title").and_then(Value::as_str).unwrap_or_default();
                let label = format!("#{number} {title}");
                let target = self.entity(&observation, kind, &id, &label, None, false);
                if let Some(actor) = self.account(&observation, item.get("user"), false) {
                    let created = timestamp(item.get("created_at"));
                    self.relation(&observation, actor, RelationKind::Authored, target.clone(), created);
                }
                self.relation(&observation, target.clone(), RelationKind::InRepository, root.clone(), None);
                Some(target)
            }
            Role::Events => {
                let root = root?;
                let actor = self.account(&observation, item.get("actor"), false)?;
                match item.get("type").and_then(Value::as_str) {
                    Some("WatchEvent") => {
                        let created = timestamp(item.get("created_at"));
                        self.relation(
                            &observation,
                            actor.clone(),
                            RelationKind::Starred,
                            root.clone(),
                            created,
                        );
                    }
                    Some("ForkEvent") => {
                        let forkee = item.get("payload").and_then(|payload| payload.get("forkee"));
                        if let Some(fork) = self.repository(&observation, forkee, false) {
                            self.relation(&observation, fork, RelationKind::ForkOf, root.clone(), None);
                        }
                    }
                    _ => {}
                }
                Some(actor)
            }
        }
    }

    /// Écrit le lot dans la transaction de l'appelant.
    ///
    /// Entités et relations sont écrites dans l'ordre de leurs clés, si bien que
    /// des workers concurrents verrouillent les lignes partagées dans le même ordre.
    pub async fn write(self, conn: &mut PgConnection) -> sqlx::Result<Written> {
        let mut written = Written::default();
        if !self.evidence.is_empty() {
            let mut hashes = Vec::with_capacity(self.evidence.len());
            let mut payloads = Vec::with_capacity(self.evidence.len());
            let mut sources = Vec::with_capacity(self.evidence.len());
            for (hash, payload, source) in self.evidence {
                hashes.push(hash);
                payloads.push(payload);
                sources.push(source);
            }
            sqlx::query(
                "INSERT INTO evidence(hash, payload, source) \
                 SELECT * FROM UNNEST($1::text[], $2::jsonb[], $3::text[]) ON CONFLICT DO NOTHING",
            )
            .bind(hashes)
            .bind(payloads)
            .bind(sources)
            .execute(&mut *conn)
            .await?;
        }

        let mut entities = self.entities;
        entities.sort_unstable_by(|a, b| (a.kind, &a.external_id).cmp(&(b.kind, &b.external_id)));
        if !entities.is_empty() {
            let ids: Vec<Uuid> = entities.iter().map(|_| Uuid::new_v4()).collect();
            let kinds: Vec<&str> = entities.iter().map(|entity| entity.kind.as_str()).collect();
            let externals: Vec<&str> = entities.iter().map(|entity| entity.external_id.as_str()).collect();
            let labels: Vec<&str> = entities.iter().map(|entity| entity.label.as_str()).collect();
            let provisional: Vec<bool> = entities.iter().map(|entity| entity.provisional).collect();
            let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
                "INSERT INTO entities(id, kind, external_id, label, provisional) \
                 SELECT * FROM UNNEST($1::uuid[], $2::text[], $3::text[], $4::text[], $5::bool[]) \
                 ON CONFLICT(kind, external_id) DO UPDATE SET label = excluded.label \
                 RETURNING id, kind, external_id",
            )
            .bind(ids)
            .bind(kinds)
            .bind(externals)
            .bind(labels)
            .bind(provisional)
            .fetch_all(&mut *conn)
            .await?;
            written.remember(rows);
        }

        // Relations may point at entities written by an earlier batch (legacy import owners).
        let mut missing: Vec<(EntityKind, String)> = self
            .relation_observations
            .iter()
            .flat_map(|relation| [&relation.source, &relation.target])
            .filter_map(|entity| match entity {
                EntityRef::Key(kind, external_id) if written.id(entity).is_none() => {
                    Some((*kind, external_id.clone()))
                }
                _ => None,
            })
            .collect();
        missing.sort_unstable();
        missing.dedup();
        if !missing.is_empty() {
            let kinds: Vec<&str> = missing.iter().map(|(kind, _)| kind.as_str()).collect();
            let externals: Vec<&str> = missing.iter().map(|(_, external_id)| external_id.as_str()).collect();
            let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
                "SELECT e.id, e.kind, e.external_id FROM entities e \
                 JOIN UNNEST($1::text[], $2::text[]) AS k(kind, external_id) USING (kind, external_id)",
            )
            .bind(kinds)
            .bind(externals)
            .fetch_all(&mut *conn)
            .await?;
            written.remember(rows);
        }

        if !self.entity_observations.is_empty() {
            let count = self.entity_observations.len();
            let (mut scans, mut entities, mut evidence, mut data, mut observed, mut precision) = (
                Vec::with_capacity(count),
                Vec::with_capacity(count),
                Vec::with_capacity(count),
                Vec::with_capacity(count),
                Vec::with_capacity(count),
                Vec::with_capacity(count),
            );
            for row in self.entity_observations {
                let Some(id) = written.ids.get(&row.entity).copied() else {
                    return Err(sqlx::Error::RowNotFound);
                };
                scans.push(row.scan_id);
                entities.push(id);
                evidence.push(row.evidence);
                data.push(row.data);
                observed.push(row.observed);
                precision.push(row.precision.as_str());
            }
            sqlx::query(
                "INSERT INTO scan_entities(scan_id, entity_id, evidence_hash, data, observed_at, time_precision) \
                 SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::jsonb[], $5::timestamptz[], $6::text[]) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(scans)
            .bind(entities)
            .bind(evidence)
            .bind(data)
            .bind(observed)
            .bind(precision)
            .execute(&mut *conn)
            .await?;
        }

        if !self.relation_observations.is_empty() {
            let mut resolved = Vec::with_capacity(self.relation_observations.len());
            for row in self.relation_observations {
                let (Some(source), Some(target)) = (written.id(&row.source), written.id(&row.target)) else {
                    return Err(sqlx::Error::RowNotFound);
                };
                resolved.push((source, target, row));
            }
            let mut keys: Vec<(Uuid, RelationKind, Uuid)> =
                resolved.iter().map(|(source, target, row)| (*source, row.kind, *target)).collect();
            keys.sort_unstable();
            keys.dedup();
            let rows: Vec<(Uuid, Uuid, String, Uuid)> = sqlx::query_as(
                "INSERT INTO relations(id, source_id, kind, target_id, context) \
                 SELECT k.*, '' FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::uuid[]) AS k \
                 ON CONFLICT(source_id, kind, target_id, context) DO UPDATE SET context = excluded.context \
                 RETURNING id, source_id, kind, target_id",
            )
            .bind(keys.iter().map(|_| Uuid::new_v4()).collect::<Vec<_>>())
            .bind(keys.iter().map(|(source, _, _)| *source).collect::<Vec<_>>())
            .bind(keys.iter().map(|(_, kind, _)| kind.as_str()).collect::<Vec<_>>())
            .bind(keys.iter().map(|(_, _, target)| *target).collect::<Vec<_>>())
            .fetch_all(&mut *conn)
            .await?;
            let relations: HashMap<(Uuid, &str, Uuid), Uuid> = rows
                .iter()
                .map(|(id, source, kind, target)| ((*source, kind.as_str(), *target), *id))
                .collect();
            let count = resolved.len();
            let (mut scans, mut ids, mut evidence, mut observed, mut event_at, mut precision) = (
                Vec::with_capacity(count),
                Vec::with_capacity(count),
                Vec::with_capacity(count),
                Vec::with_capacity(count),
                Vec::with_capacity(count),
                Vec::with_capacity(count),
            );
            for (source, target, row) in resolved {
                let Some(id) = relations.get(&(source, row.kind.as_str(), target)).copied() else {
                    return Err(sqlx::Error::RowNotFound);
                };
                scans.push(row.scan_id);
                ids.push(id);
                evidence.push(row.evidence);
                observed.push(row.observed);
                event_at.push(row.event_at);
                precision.push(row.precision.as_str());
            }
            sqlx::query(
                "INSERT INTO scan_relations(scan_id, relation_id, evidence_hash, observed_at, source_event_at, \
                 time_precision) \
                 SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::timestamptz[], $5::timestamptz[], \
                 $6::text[]) ON CONFLICT DO NOTHING",
            )
            .bind(scans)
            .bind(ids)
            .bind(evidence)
            .bind(observed)
            .bind(event_at)
            .bind(precision)
            .execute(&mut *conn)
            .await?;
        }
        Ok(written)
    }
}

impl Written {
    fn remember(&mut self, rows: Vec<(Uuid, String, String)>) {
        for (id, kind, external_id) in rows {
            if let Some(kind) = EntityKind::parse(&kind) {
                self.ids.insert((kind, external_id), id);
            }
        }
    }
}

/// Enregistre un seul objet ; pour une page entière, préparer un [`Batch`].
pub async fn ingest_github(
    conn: &mut PgConnection,
    scan_id: Uuid,
    role: Role,
    item: &Value,
    source_url: &str,
    observed: DateTime<Utc>,
    root: Option<Uuid>,
) -> sqlx::Result<Option<Uuid>> {
    let mut batch = Batch::default();
    let root = root.map(EntityRef::Id);
    let entity = batch.ingest_github(scan_id, role, item, source_url, observed, root.as_ref());
    let written = batch.write(conn).await?;
    Ok(entity.and_then(|entity| written.id(&entity)))
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

    #[test]
    fn a_batch_behaves_like_successive_writes() {
        let scan = Uuid::nil();
        let mut batch = Batch::default();
        let first = json!({"id": 1, "login": "old-name"});
        let second = json!({"id": 1, "login": "new-name"});
        let proof = batch.evidence(&first, "source");
        assert_eq!(batch.evidence(&first, "source"), proof, "the same proof is prepared once");
        let observation =
            Observation { scan_id: scan, proof: &proof, observed: None, precision: Precision::Unknown };
        let account = batch.account(&observation, Some(&first), true);
        batch.account(&observation, Some(&second), true);
        batch.entity(&observation, EntityKind::Repository, "legacy:x", "x/y", None, true);
        batch.entity(&observation, EntityKind::Repository, "legacy:x", "x/z", None, false);
        assert_eq!(account, Some(EntityRef::Key(EntityKind::Account, "1".into())));
        assert_eq!(batch.evidence.len(), 1);
        assert_eq!(batch.entities.len(), 2);
        assert_eq!(batch.entities[0].label, "new-name", "the last label wins");
        assert!(
            batch.entities[1].provisional && batch.entities[1].label == "x/z",
            "provisional state is kept"
        );
        assert_eq!(batch.entity_observations.len(), 4, "each observation is kept");
        assert!(!batch.is_empty());
        assert_eq!(
            Batch::default().ingest_github(scan, Role::Issues, &json!({"id": 1}), "s", Utc::now(), None),
            None
        );
        assert_eq!(EntityKind::parse("PullRequest"), Some(EntityKind::PullRequest));
        assert_eq!(EntityKind::parse("Team"), None);
        assert_eq!(Written::default().id(&EntityRef::Id(scan)), Some(scan));
    }
}
