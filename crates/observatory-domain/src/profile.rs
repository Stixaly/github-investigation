//! Profils de collecte : réglages versionnés et immuables d'un lancement.
//!
//! Un scan garde le nom, la version et une copie des réglages de son profil :
//! modifier un profil, c'est en publier une nouvelle version, et les scans
//! déjà lancés restent lisibles avec les réglages qui les ont produits.

use serde::{Deserialize, Serialize};

/// Relations qu'un profil peut demander après la seed (`repositories` : dépôts d'un compte).
pub const RELATIONS: &[&str] = &["contributors", "forks", "issues", "events", "repositories"];
/// Budget de requêtes maximal d'un scan, tous profils confondus.
pub const MAX_REQUESTS: i32 = 50;
/// Pages lues au plus par endpoint, tous profils confondus.
pub const MAX_PAGES: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionSettings {
    /// Endpoints de relations lus après la seed, parmi [`RELATIONS`].
    pub relations: Vec<String>,
    /// Pages lues au plus par endpoint, reprises comprises.
    pub max_pages: u32,
    /// Budget proposé par défaut, en requêtes décomptées du quota GitHub.
    pub default_requests: i32,
    /// Budget accepté au plus pour ce profil.
    pub max_requests: i32,
}

impl CollectionSettings {
    /// Réglages de la 0.3 (profil « reseau » v1), appliqués aux scans lancés sans profil.
    pub fn network() -> Self {
        Self {
            relations: RELATIONS.iter().map(|relation| (*relation).to_owned()).collect(),
            max_pages: 2,
            default_requests: 12,
            max_requests: MAX_REQUESTS,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut seen = Vec::new();
        for relation in &self.relations {
            if !RELATIONS.contains(&relation.as_str()) || seen.contains(&relation) {
                return Err(format!("relation inconnue ou répétée « {relation} »"));
            }
            seen.push(relation);
        }
        if !(1..=MAX_PAGES).contains(&self.max_pages) {
            return Err(format!("1 à {MAX_PAGES} pages par endpoint"));
        }
        if !(1..=self.max_requests).contains(&self.default_requests) || self.max_requests > MAX_REQUESTS {
            return Err(format!(
                "budget par défaut entre 1 et le plafond, plafond de {MAX_REQUESTS} au plus"
            ));
        }
        Ok(())
    }

    pub fn wants(&self, relation: &str) -> bool {
        self.relations.iter().any(|wanted| wanted == relation)
    }

    /// Budget effectif d'un lancement : celui demandé, sinon celui du profil.
    pub fn budget(&self, requested: Option<i32>) -> Result<i32, String> {
        let budget = requested.unwrap_or(self.default_requests);
        if (1..=self.max_requests).contains(&budget) {
            Ok(budget)
        } else {
            Err(format!("Budget : 1 à {} requêtes pour ce profil", self.max_requests))
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_network_profile_is_the_0_3_collection() {
        let network = CollectionSettings::network();
        assert_eq!(network.validate(), Ok(()));
        assert!(RELATIONS.iter().all(|relation| network.wants(relation)));
        assert_eq!(
            (network.budget(None), network.budget(Some(50)), network.budget(Some(51)).is_err()),
            (Ok(12), Ok(50), true)
        );
    }

    #[test]
    fn settings_are_bounded() {
        let parse = |value| serde_json::from_value::<CollectionSettings>(value).unwrap();
        let preview = parse(
            json!({"relations": ["contributors", "forks"], "max_pages": 1, "default_requests": 6, "max_requests": 12}),
        );
        assert_eq!(preview.validate(), Ok(()));
        assert!(!preview.wants("events"));
        assert_eq!(preview.budget(Some(24)), Err("Budget : 1 à 12 requêtes pour ce profil".into()));
        let mut wrong = preview.clone();
        wrong.relations.push("stargazers".into());
        assert!(wrong.validate().is_err());
        let mut repeated = preview.clone();
        repeated.relations.push("forks".into());
        assert!(repeated.validate().is_err());
        assert!(CollectionSettings { max_pages: 0, ..preview.clone() }.validate().is_err());
        assert!(CollectionSettings { default_requests: 13, ..preview.clone() }.validate().is_err());
        assert!(CollectionSettings { max_requests: 60, default_requests: 6, ..preview }.validate().is_err());
        assert!(
            serde_json::from_value::<CollectionSettings>(json!({"relations": [], "max_pages": 1,
            "default_requests": 1, "max_requests": 1, "depth": 3}))
            .is_err(),
            "unknown settings are refused"
        );
    }
}
