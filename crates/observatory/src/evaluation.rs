//! Évaluation des listes de termes sur les observations d'un dossier.
//!
//! Une évaluation est une tâche du worker : réservation `SKIP LOCKED`, lease et
//! génération comme les scans. Elle lit les observations enregistrées avant
//! son démarrage (manifeste figé), par lots d'entités dans l'ordre de leur
//! identifiant ; chaque lot écrit ses résultats, alimente le triage et avance
//! le curseur de reprise dans une seule transaction.

use chrono::{DateTime, Utc};
use observatory_domain::canonical::sha256_hex;
use observatory_domain::rules::{Definition, Hit, Outcome, RuleResult, observation_input};
use serde_json::{Map, Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

/// Entités évaluées par transaction.
pub const BATCH: i64 = 500;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Run {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub list_id: Uuid,
    pub list_version: i32,
    pub status: String,
    pub manifest_at: Option<DateTime<Utc>>,
    pub cursor: Option<Uuid>,
    pub counts: Value,
    pub generation: i32,
}

/// Réserve la plus ancienne évaluation en attente, ou reprend une évaluation dont le lease a expiré.
pub async fn claim(pool: &PgPool) -> sqlx::Result<Option<Run>> {
    let mut tx = pool.begin().await?;
    let id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM rule_runs WHERE status = 'queued' OR (status = 'running' AND lease_until < now()) \
         ORDER BY requested_at FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(id) = id else {
        tx.commit().await?;
        return Ok(None);
    };
    let run = sqlx::query_as(
        "UPDATE rule_runs SET status = 'running', generation = generation + 1, \
         started_at = coalesce(started_at, now()), manifest_at = coalesce(manifest_at, now()), \
         lease_until = now() + interval '90 seconds' WHERE id = $1 \
         RETURNING id, investigation_id, list_id, list_version, status, manifest_at, cursor, counts, generation",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(run))
}

/// Vrai si l'évaluation appartient encore à ce worker ; prolonge son lease.
async fn guard(conn: &mut PgConnection, run: &Run) -> sqlx::Result<bool> {
    let row: Option<(String, i32)> =
        sqlx::query_as("SELECT status, generation FROM rule_runs WHERE id = $1 FOR UPDATE")
            .bind(run.id)
            .fetch_optional(&mut *conn)
            .await?;
    if row != Some(("running".to_owned(), run.generation)) {
        return Ok(false);
    }
    sqlx::query("UPDATE rule_runs SET lease_until = now() + interval '90 seconds' WHERE id = $1")
        .bind(run.id)
        .execute(conn)
        .await?;
    Ok(true)
}

pub async fn finish(pool: &PgPool, run: &Run, status: &str, error: Option<&str>) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE rule_runs SET status = $1, error = $2, finished_at = now(), lease_until = NULL \
         WHERE id = $3 AND status = 'running' AND generation = $4",
    )
    .bind(status)
    .bind(error)
    .bind(run.id)
    .bind(run.generation)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Rend immédiatement une évaluation interrompue par l'arrêt du worker.
pub async fn release(pool: &PgPool, run: &Run) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE rule_runs SET status = 'queued', lease_until = NULL, generation = generation + 1 \
         WHERE id = $1 AND status = 'running' AND generation = $2",
    )
    .bind(run.id)
    .bind(run.generation)
    .execute(pool)
    .await
    .map(|_| ())
}

#[derive(Debug, sqlx::FromRow)]
struct Observation {
    id: Uuid,
    kind: String,
    data: Value,
    evidence_hash: String,
    observed_at: Option<DateTime<Utc>>,
}

/// Résumé des occurrences : une tâche de triage n'est rouverte que s'il change.
fn signature(hits: &[Hit]) -> String {
    let mut parts: Vec<Value> =
        hits.iter().map(|hit| json!([hit.field, hit.index, hit.term, hit.excerpt])).collect();
    parts.sort_by_key(ToString::to_string);
    sha256_hex(&Value::Array(parts).to_string())
}

fn outcome_name(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Match => "match",
        Outcome::NoMatch => "no_match",
        Outcome::NotEvaluated => "not_evaluated",
    }
}

fn value<T: serde::Serialize>(item: &T) -> Value {
    serde_json::to_value(item).unwrap_or(Value::Null)
}

fn add(counts: &mut Map<String, Value>, key: &str, by: u64) {
    let current = counts.get(key).and_then(Value::as_u64).unwrap_or(0);
    counts.insert(key.to_owned(), (current + by).into());
}

/// Exécute une évaluation réservée, lot après lot, jusqu'à la fin ou la perte du lease.
pub async fn execute(pool: &PgPool, run: &Run) -> sqlx::Result<()> {
    let stored: Option<Value> = sqlx::query_scalar(
        "SELECT definition FROM keyword_list_versions WHERE list_id = $1 AND version = $2",
    )
    .bind(run.list_id)
    .bind(run.list_version)
    .fetch_optional(pool)
    .await?;
    let Some(definition) = stored.and_then(|stored| Definition::parse(&stored).ok()) else {
        return finish(pool, run, "failed", Some("Liste illisible par ce moteur")).await;
    };
    let manifest = run.manifest_at.unwrap_or_else(Utc::now);
    let mut cursor = run.cursor;
    let mut counts = run.counts.as_object().cloned().unwrap_or_default();
    loop {
        let mut tx = pool.begin().await?;
        if !guard(&mut tx, run).await? {
            return Ok(());
        }
        let batch: Vec<Observation> = sqlx::query_as(
            "SELECT DISTINCT ON (e.id) e.id, e.kind, coalesce(se.data, ev.payload) AS data, se.evidence_hash, \
             se.observed_at \
             FROM scan_entities se JOIN scans s ON s.id = se.scan_id JOIN entities e ON e.id = se.entity_id \
             JOIN evidence ev ON ev.hash = se.evidence_hash \
             WHERE s.investigation_id = $1 AND se.recorded_at <= $2 AND e.kind IN ('Repository', 'Account') \
               AND ($3::uuid IS NULL OR e.id > $3) \
             ORDER BY e.id, se.recorded_at DESC, se.evidence_hash LIMIT $4",
        )
        .bind(run.investigation_id)
        .bind(manifest)
        .bind(cursor)
        .bind(BATCH)
        .fetch_all(&mut *tx)
        .await?;
        let Some(last) = batch.last().map(|observed| observed.id) else {
            tx.commit().await?;
            return finish(pool, run, "succeeded", None).await;
        };
        let mut matches: Vec<(&Observation, RuleResult)> = Vec::new();
        for observed in &batch {
            let Some((kind, input)) = observation_input(&observed.kind, &observed.data) else {
                continue;
            };
            add(&mut counts, "entities", 1);
            for result in definition.evaluate_kind(kind, &input) {
                add(&mut counts, outcome_name(result.outcome), 1);
                if result.outcome != Outcome::NoMatch {
                    matches.push((observed, result));
                }
            }
        }
        write_matches(&mut tx, run, &matches).await?;
        sqlx::query("UPDATE rule_runs SET cursor = $1, counts = $2 WHERE id = $3")
            .bind(last)
            .bind(Value::Object(counts.clone()))
            .bind(run.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        cursor = Some(last);
    }
}

/// Écrit les résultats d'un lot et met à jour la file de triage, en deux requêtes ensemblistes.
async fn write_matches(
    conn: &mut PgConnection,
    run: &Run,
    matches: &[(&Observation, RuleResult)],
) -> sqlx::Result<()> {
    if matches.is_empty() {
        return Ok(());
    }
    let entities: Vec<Uuid> = matches.iter().map(|(observed, _)| observed.id).collect();
    let rules: Vec<&str> = matches.iter().map(|(_, result)| result.rule.as_str()).collect();
    let outcomes: Vec<&str> = matches.iter().map(|(_, result)| outcome_name(result.outcome)).collect();
    let priorities: Vec<String> = matches
        .iter()
        .map(|(_, result)| value(&result.review_priority).as_str().unwrap_or("normal").to_owned())
        .collect();
    let hits: Vec<Value> = matches.iter().map(|(_, result)| value(&result.hits)).collect();
    let exclusions: Vec<Value> = matches.iter().map(|(_, result)| value(&result.exclusions)).collect();
    let missing: Vec<Value> = matches.iter().map(|(_, result)| value(&result.not_evaluated)).collect();
    let evidence: Vec<&str> = matches.iter().map(|(observed, _)| observed.evidence_hash.as_str()).collect();
    let observed_at: Vec<Option<DateTime<Utc>>> =
        matches.iter().map(|(observed, _)| observed.observed_at).collect();
    sqlx::query(
        "INSERT INTO rule_matches(run_id, entity_id, rule_id, outcome, review_priority, hits, exclusions, \
         not_evaluated, evidence_hash, observed_at) \
         SELECT $1, * FROM UNNEST($2::uuid[], $3::text[], $4::text[], $5::text[], $6::jsonb[], $7::jsonb[], \
           $8::jsonb[], $9::text[], $10::timestamptz[]) \
         ON CONFLICT DO NOTHING",
    )
    .bind(run.id)
    .bind(&entities)
    .bind(&rules)
    .bind(&outcomes)
    .bind(&priorities)
    .bind(&hits)
    .bind(&exclusions)
    .bind(&missing)
    .bind(&evidence)
    .bind(&observed_at)
    .execute(&mut *conn)
    .await?;
    let found: Vec<usize> =
        (0..matches.len()).filter(|&index| matches[index].1.outcome == Outcome::Match).collect();
    if found.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO triage_items(id, investigation_id, entity_id, list_id, rule_id, signature, evidence_hash, last_run_id) \
         SELECT u.id, $1, u.entity, $2, u.rule, u.signature, u.evidence, $3 \
         FROM UNNEST($4::uuid[], $5::uuid[], $6::text[], $7::text[], $8::text[]) AS u(id, entity, rule, signature, evidence) \
         ON CONFLICT (investigation_id, entity_id, list_id, rule_id) DO UPDATE SET \
           state = CASE WHEN triage_items.signature <> excluded.signature \
                          AND triage_items.state IN ('retained', 'dismissed') THEN 'new' ELSE triage_items.state END, \
           updated_at = CASE WHEN triage_items.signature <> excluded.signature THEN now() ELSE triage_items.updated_at END, \
           signature = excluded.signature, evidence_hash = excluded.evidence_hash, last_run_id = excluded.last_run_id",
    )
    .bind(run.investigation_id)
    .bind(run.list_id)
    .bind(run.id)
    .bind(found.iter().map(|_| Uuid::new_v4()).collect::<Vec<_>>())
    .bind(found.iter().map(|&index| matches[index].0.id).collect::<Vec<_>>())
    .bind(found.iter().map(|&index| matches[index].1.rule.as_str()).collect::<Vec<_>>())
    .bind(found.iter().map(|&index| signature(&matches[index].1.hits)).collect::<Vec<_>>())
    .bind(found.iter().map(|&index| matches[index].0.evidence_hash.as_str()).collect::<Vec<_>>())
    .execute(conn)
    .await
    .map(|_| ())
}
