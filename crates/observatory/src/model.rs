//! Lignes de la base exposées par l'API ou partagées entre l'API et le worker.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Investigation {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Scan {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub requested_by: Uuid,
    pub source: String,
    pub seed: String,
    pub status: String,
    pub max_requests: i32,
    pub requests_used: i32,
    pub requested_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub available_at: DateTime<Utc>,
    pub lease_until: Option<DateTime<Utc>>,
    pub generation: i32,
    pub attempts: i32,
    pub coverage: Value,
    pub error: Option<String>,
    pub import_key: Option<String>,
    pub idempotency_key: Uuid,
    /// Profil de collecte du lancement ; absent pour les scans antérieurs à la 0.4.
    pub profile_name: Option<String>,
    pub profile_version: Option<i32>,
    /// Copie des réglages du profil au lancement.
    pub profile: Option<Value>,
    /// Plan de requêtes d'une découverte (source `search`).
    pub plan: Option<Value>,
}

/// États d'un scan encore susceptibles de consommer l'API GitHub.
pub const PENDING_STATUSES: [&str; 3] = ["queued", "running", "waiting_quota"];
