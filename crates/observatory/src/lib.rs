//! Observatory : investigation des relations entre comptes et dépôts GitHub publics.
//!
//! Un seul binaire, plusieurs rôles (`observatory serve|worker|migrate|...`) :
//! l'API et le worker partagent la même image Alpine mais tournent dans des
//! conteneurs distincts, avec des secrets distincts.

pub mod api;
pub mod collector;
pub mod config;
pub mod db;
pub mod demo;
pub mod error;
pub mod evaluation;
pub mod legacy;
pub mod metrics;
pub mod migrate;
pub mod model;
pub mod security;
pub mod store;
pub mod users;
pub mod worker;
