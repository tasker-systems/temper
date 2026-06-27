//! General API types — health, events, search, profile updates.

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

/// Response body for the event-cursor endpoint: the most recent event id
/// recorded for a context, or `None` if the context has no events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct EventCursorResponse {
    /// Most recent `kb_events.id` for the context, newest by `created`.
    pub latest_event_id: Option<Uuid>,
}

/// Default search config for full-text search.
fn default_search_config() -> String {
    "english".to_string()
}

/// Default for graph_expand — true enables graph-enhanced search.
fn default_graph_expand() -> bool {
    true
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
    ///
    /// NOTE: reserved/inert in Surface A — FTS is hardcoded `'english'` in `search_fts_candidates`
    /// (Beat 1 kept multilingual storage-only); this param does not affect results yet.
    #[serde(default = "default_search_config")]
    pub search_config: String,
    /// Filter by context **ref** (UUID or decorated @owner/slug), resolved server-side.
    pub context_ref: Option<String>,
    /// Filter by document type.
    pub doc_type: Option<String>,
    /// Maximum results (default 10, max 50).
    pub limit: Option<i64>,
    /// Offset for pagination.
    #[serde(default)]
    pub offset: Option<i64>,
    /// Explicit seed resource IDs for graph expansion.
    #[serde(default)]
    pub seed_ids: Option<Vec<Uuid>>,
    /// Edge type filter for graph expansion (empty = all types).
    #[serde(default)]
    pub edge_types: Option<Vec<String>>,
    /// Max hops for graph traversal (default 2, max 3 — clamped for Surface A).
    #[serde(default)]
    pub graph_depth: Option<i32>,
    /// Whether to expand results via graph edges (default true).
    #[serde(default = "default_graph_expand")]
    pub graph_expand: bool,
}

impl Default for SearchParams {
    fn default() -> Self {
        Self {
            embedding: None,
            query: None,
            search_config: default_search_config(),
            context_ref: None,
            doc_type: None,
            limit: None,
            offset: None,
            seed_ids: None,
            edge_types: None,
            graph_depth: None,
            graph_expand: default_graph_expand(),
        }
    }
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
    /// Surface A (Beat 2) structural-proximity score: max-over-paths γ^hop·Π edge_weight, 0 when the
    /// candidate was reached only by FTS/vector. Exposed so the graph term is observable for tuning.
    pub graph_score: f32,
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
        assert!(params.context_ref.is_none());
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
    fn search_params_graph_expand_defaults_true() {
        let json = r#"{"query": "hello"}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert!(params.graph_expand);
        assert!(params.seed_ids.is_none());
        assert!(params.edge_types.is_none());
        assert!(params.graph_depth.is_none());
    }

    #[test]
    fn search_params_graph_expand_can_be_disabled() {
        let json = r#"{"query": "hello", "graph_expand": false}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert!(!params.graph_expand);
    }

    #[test]
    fn search_params_with_seed_ids() {
        let json = r#"{"seed_ids": ["019d1d24-2000-7379-8f26-ae4ae87bc5c6"]}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.seed_ids.unwrap().len(), 1);
        assert!(params.query.is_none());
    }

    #[test]
    fn search_params_with_edge_types_and_depth() {
        let json =
            r#"{"query": "test", "edge_types": ["extends", "depends_on"], "graph_depth": 4}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.edge_types.unwrap(), vec!["extends", "depends_on"]);
        assert_eq!(params.graph_depth.unwrap(), 4);
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

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn event_cursor_response_round_trips_some_and_none() {
        let some = EventCursorResponse {
            latest_event_id: Some(Uuid::nil()),
        };
        let none = EventCursorResponse {
            latest_event_id: None,
        };
        for value in [some, none] {
            let json = serde_json::to_string(&value).unwrap();
            let back: EventCursorResponse = serde_json::from_str(&json).unwrap();
            assert_eq!(back.latest_event_id, value.latest_event_id);
        }
    }
}
