//! Règles pures d'Observatory, sans réseau ni base de données.
//!
//! Tout ce qui lit une entrée contrôlée par un tiers (seed saisie par un
//! analyste, en-tête `Link` renvoyé par GitHub, JSON importé) est ici : ces
//! fonctions sont testées unitairement et soumises au fuzzing (`fuzz/`).

pub mod canonical;
pub mod pagination;
pub mod profile;
pub mod rules;
pub mod seed;
pub mod snapshot;
pub mod text;

pub use canonical::evidence_hash;
pub use pagination::{GITHUB_API, next_page};
pub use seed::{Seed, SeedError, SeedKind, parse_seed};
