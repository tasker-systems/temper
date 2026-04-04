//! Search tool — full-text vector search across resources.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::service::TemperMcpService;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchInput {
    /// Pre-computed 768-dimensional embedding vector for the search query.
    #[schemars(description = "768-dimensional embedding vector for semantic search")]
    pub embedding: Vec<f32>,
    /// Optional context name to scope the search.
    #[schemars(description = "Optional context name to filter search results")]
    pub context_name: Option<String>,
    /// Optional document type filter.
    #[schemars(description = "Optional document type to filter results (e.g. 'task', 'session')")]
    pub doc_type: Option<String>,
    /// Maximum results (default 10, max 50).
    #[schemars(description = "Maximum number of results to return (default 10, max 50)")]
    pub limit: Option<i64>,
}

pub async fn search(
    svc: &TemperMcpService,
    input: SearchInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let params = temper_core::types::api::SearchParams {
        embedding: input.embedding,
        context_name: input.context_name,
        doc_type: input.doc_type,
        limit: input.limit,
    };

    match temper_api::services::search_service::search(pool, profile.id, params).await {
        Ok(rows) => {
            let items: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "resource_id": r.resource_id,
                        "title": r.title,
                        "kb_uri": r.kb_uri,
                        "origin_uri": r.origin_uri,
                        "context": r.context,
                        "doc_type": r.doc_type,
                        "score": r.score,
                        "snippet": r.snippet,
                        "header_path": r.header_path,
                    })
                })
                .collect();

            let text = serde_json::to_string_pretty(&items)
                .unwrap_or_else(|_| "[]".to_string());
            Ok(CallToolResult::success(vec![
                rmcp::model::Content::text(text),
            ]))
        }
        Err(e) => Err(rmcp::ErrorData::internal_error(
            format!("Search failed: {e}"),
            None,
        )),
    }
}
