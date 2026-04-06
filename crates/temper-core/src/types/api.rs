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
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "event.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct EventRow {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub device_id: String,
    pub kb_context_id: Option<Uuid>,
    pub resource_id: Option<Uuid>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created: DateTime<Utc>,
}

/// Query parameters for listing events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
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

/// Default search config for full-text search.
fn default_search_config() -> String {
    "english".to_string()
}

/// Request body for POST /api/search.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct SearchParams {
    /// Pre-computed 768-dim embedding vector.
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
    /// Plain-text query for full-text search.
    #[serde(default)]
    pub query: Option<String>,
    /// Postgres text-search configuration (default "english").
    #[serde(default = "default_search_config")]
    pub search_config: String,
    /// Filter by context name (resolved to UUID server-side).
    pub context_name: Option<String>,
    /// Filter by document type.
    pub doc_type: Option<String>,
    /// Maximum results (default 10, max 50).
    pub limit: Option<i64>,
    /// Offset for pagination.
    #[serde(default)]
    pub offset: Option<i64>,
}

/// A single search result.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
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

/// A unified search result combining FTS and vector scores.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct UnifiedSearchResultRow {
    pub resource_id: Uuid,
    pub title: String,
    pub slug: String,
    pub kb_uri: String,
    pub origin_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub doc_type: String,
    pub fts_score: f32,
    pub vector_score: f32,
    pub combined_score: f32,
    pub origin: String,
}

/// Request body for updating a profile.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ProfileUpdateRequest {
    pub display_name: Option<String>,
    pub preferences: Option<serde_json::Value>,
    pub vault_config: Option<VaultConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_params_deserializes_query_only() {
        let json = r#"{"query": "hello world"}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.query.as_deref(), Some("hello world"));
        assert!(params.embedding.is_none());
        assert_eq!(params.search_config, "english");
        assert!(params.context_name.is_none());
        assert!(params.doc_type.is_none());
        assert!(params.limit.is_none());
        assert!(params.offset.is_none());
    }

    #[test]
    fn search_params_deserializes_embedding_only() {
        let json = r#"{"embedding": [0.1, 0.2, 0.3]}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert!(params.query.is_none());
        assert_eq!(params.embedding.unwrap(), vec![0.1, 0.2, 0.3]);
        assert_eq!(params.search_config, "english");
    }

    #[test]
    fn search_params_deserializes_both() {
        let json = r#"{
            "query": "test query",
            "embedding": [1.0, 2.0],
            "search_config": "simple"
        }"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.query.as_deref(), Some("test query"));
        assert_eq!(params.embedding.unwrap(), vec![1.0, 2.0]);
        assert_eq!(params.search_config, "simple");
    }
}
