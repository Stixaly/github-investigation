//! Exécution d'une découverte : requêtes de recherche, pages, subdivisions et candidats.
//!
//! Chaque requête du plan est une étape avec son point de reprise. Une requête
//! saturée (plus de résultats que les pages permises ou que les 1 000 accessibles)
//! est subdivisée tant que le plan le permet ; ses sous-requêtes sont ajoutées
//! aux étapes, enregistrées dans la couverture avant que la requête d'origine
//! soit close, si bien qu'une reprise les retrouve.

use chrono::Utc;
use observatory_domain::discovery::{MAX_RESULTS, PER_PAGE, Plan, Query, SearchKind, search_url};
use observatory_domain::pagination::next_page;
use serde_json::{Map, Value};

use super::{Batch, CollectError, Collector, Role, count, guard, increment, save_checkpoint};

impl Collector<'_> {
    /// Découverte : chaque requête du plan, puis ses subdivisions. Retourne `succeeded` ou `partial`.
    pub(super) async fn discover(&self) -> Result<&'static str, CollectError> {
        let plan: Plan = self
            .scan
            .plan
            .clone()
            .and_then(|plan| serde_json::from_value(plan).ok())
            .ok_or_else(|| CollectError::Fetch("Plan de découverte illisible".into()))?;
        if plan.kind == SearchKind::Code && !self.github.authenticated() {
            return Err(CollectError::Fetch(
                "La recherche de code exige un token GitHub configuré sur le worker".into(),
            ));
        }
        let mut coverage = self.scan.coverage.as_object().cloned().unwrap_or_default();
        let previous = coverage.get("stats").cloned();
        let mut steps: Vec<Query> = coverage
            .get("steps")
            .cloned()
            .and_then(|steps| serde_json::from_value(steps).ok())
            .unwrap_or_else(|| plan.queries.clone());
        let mut subdivisions = count(&coverage, "subdivisions");
        let mut checkpoints = self.checkpoints().await?;
        let today = Utc::now().timestamp().div_euclid(86_400);
        let reachable = (u64::from(plan.max_pages) * PER_PAGE).min(MAX_RESULTS);
        let mut index = 0;
        while index < steps.len() {
            let query = steps[index].clone();
            index += 1;
            let name = format!("q{index}");
            let checkpoint = checkpoints.remove(&name);
            if checkpoint.as_ref().is_some_and(|checkpoint| checkpoint.done) {
                continue;
            }
            let mut result = Map::new();
            result.insert("query".into(), query.q.clone().into());
            result.insert("origin".into(), query.origin.clone().into());
            result.insert("status".into(), "partial".into());
            result.insert("items".into(), 0.into());
            result.insert("pages".into(), 0.into());
            let mut next = Some(search_url(self.github.base(), plan.kind, &query.q));
            if let Some(checkpoint) = checkpoint {
                result.extend(checkpoint.state.as_object().cloned().unwrap_or_default());
                next = checkpoint.next_url;
            }
            let outcome = self.search_pages(plan.kind, &name, &mut next, plan.max_pages, &mut result).await;
            let total = result.get("total_count").and_then(Value::as_u64).unwrap_or(0);
            if outcome.is_ok() && total > reachable {
                let halves = (subdivisions < u64::from(plan.max_subdivisions))
                    .then(|| query.subdivide(plan.kind, today))
                    .flatten();
                if let Some(halves) = halves {
                    subdivisions += 1;
                    steps.extend(halves);
                    result.insert("status".into(), "subdivided".into());
                    result.insert(
                        "reason".into(),
                        format!("{total} résultats : subdivisée en deux requêtes").into(),
                    );
                } else {
                    result.insert("status".into(), "partial".into());
                    result.insert(
                        "reason".into(),
                        format!("{total} résultats : seuls les {reachable} premiers sont accessibles").into(),
                    );
                }
            } else if outcome.is_ok() && result["status"] == "partial" {
                result.insert("reason".into(), "Plafond de pages atteint".into());
            }
            if result.get("incomplete_results") == Some(&Value::Bool(true)) && result["status"] == "complete"
            {
                result.insert("status".into(), "partial".into());
                result.insert(
                    "reason".into(),
                    "GitHub signale une réponse incomplète (recherche interrompue)".into(),
                );
            }
            let stop = match outcome {
                Ok(()) => None,
                Err(CollectError::BudgetExceeded) => {
                    result.insert("reason".into(), "Budget de requêtes atteint".into());
                    None
                }
                Err(CollectError::Fetch(reason)) => {
                    result.insert("status".into(), "error".into());
                    result.insert("reason".into(), reason.into());
                    None
                }
                Err(other) => Some(other),
            };
            coverage.insert(name.clone(), Value::Object(result));
            coverage.insert("steps".into(), serde_json::to_value(&steps).unwrap_or_default());
            coverage.insert("subdivisions".into(), subdivisions.into());
            let finished = stop.is_none().then_some(name.as_str());
            self.save_coverage(&mut coverage, previous.as_ref(), finished).await?;
            if let Some(error) = stop {
                return Err(error);
            }
        }
        let partial = coverage
            .values()
            .any(|entry| matches!(entry.get("status").and_then(Value::as_str), Some("partial" | "error")));
        Ok(if partial { "partial" } else { "succeeded" })
    }

    /// Pages d'une requête de recherche ; chaque page est enregistrée avec son point de reprise.
    async fn search_pages(
        &self,
        kind: SearchKind,
        step: &str,
        next: &mut Option<String>,
        max_pages: u32,
        result: &mut Map<String, Value>,
    ) -> Result<(), CollectError> {
        while let Some(url) = next.clone() {
            if count(result, "pages") >= u64::from(max_pages) {
                return Ok(());
            }
            let page = self.get_on(&url, kind.resource()).await?;
            let Some(items) = page.payload.get("items").and_then(Value::as_array) else {
                return Err(CollectError::Fetch("Réponse de recherche attendue".into()));
            };
            let following = next_page(&page.link, &url, None, self.github.base());
            let mut progress = result.clone();
            increment(&mut progress, "items", items.len());
            increment(&mut progress, "pages", 1);
            if let Some(total) = page.payload.get("total_count").filter(|total| total.is_u64()) {
                progress.insert("total_count".into(), total.clone());
            }
            if page.payload.get("incomplete_results") == Some(&Value::Bool(true)) {
                progress.insert("incomplete_results".into(), true.into());
            }
            if matches!(following, Ok(None)) {
                progress.insert("status".into(), "complete".into());
            }
            let following_url = following.as_ref().ok().cloned().flatten();
            let mut batch = Batch::default();
            for item in items {
                match kind {
                    SearchKind::Repositories => {
                        batch.ingest_github(
                            self.scan.id,
                            Role::Repositories,
                            item,
                            &url,
                            page.observed,
                            None,
                        );
                    }
                    SearchKind::Code => {
                        // The repository is the candidate; the hit (path, file hash) is its evidence.
                        // Fragments are never requested while content acquisition is off.
                        let Some(repository) =
                            item.get("repository").filter(|repository| repository.is_object())
                        else {
                            continue;
                        };
                        let mut proof = item.clone();
                        if let Some(object) = proof.as_object_mut() {
                            object.remove("text_matches");
                        }
                        batch.ingest_received(
                            self.scan.id,
                            Role::Repositories,
                            repository,
                            (&proof, &url),
                            page.observed,
                            None,
                        );
                    }
                }
            }
            let mut tx = self.pool.begin().await?;
            guard(&mut tx, self.scan).await?;
            batch.write(&mut tx).await?;
            let state = Value::Object(progress.clone());
            save_checkpoint(&mut tx, self.scan.id, step, following_url.as_deref(), &state, false).await?;
            tx.commit().await?;
            *result = progress;
            *next = following_url;
            following?;
        }
        Ok(())
    }
}
