//! General API types — health, events, search, profile updates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::types::vault_config::VaultConfig;

/// Response body for the health endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

/// Row type matching the `kb_events` table.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct EventRow {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub client_id: String,
    pub kb_context_id: Option<Uuid>,
    pub resource_id: Option<Uuid>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created: DateTime<Utc>,
}

/// Query parameters for listing events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct EventListParams {
    /// Filter by resource ID.
    pub resource_id: Option<Uuid>,
    /// Filter by event type.
    pub event_type: Option<String>,
    /// Maximum results to return (default 50, max 200).
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// Request body for POST /api/search.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct SearchParams {
    /// Pre-computed 768-dim embedding vector.
    pub embedding: Vec<f32>,
    /// Filter by kb_context ID.
    pub context: Option<Uuid>,
    /// Filter by document type.
    pub doc_type: Option<String>,
    /// Maximum results (default 10, max 50).
    pub limit: Option<i64>,
}

/// A single search result.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct SearchResultRow {
    pub resource_id: Uuid,
    pub title: String,
    /// Canonical kb:// URI: kb://context/doc_type/uuid (from kb_resource_uri SQL function)
    pub kb_uri: String,
    /// Original source URL or file reference
    pub origin_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub doc_type: String,
    pub score: f32,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_path: Option<String>,
}

/// Request body for updating a profile.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ProfileUpdateRequest {
    pub display_name: Option<String>,
    pub preferences: Option<serde_json::Value>,
    pub vault_config: Option<VaultConfig>,
}
