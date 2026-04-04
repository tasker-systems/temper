//! Context tools — list and inspect knowledge base contexts.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use crate::service::TemperMcpService;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListContextsInput {}

pub async fn list_contexts(
    svc: &TemperMcpService,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    match temper_api::services::context_service::list_visible(pool, profile.id).await {
        Ok(rows) => {
            let items: Vec<serde_json::Value> = rows
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "id": c.id,
                        "name": c.name,
                        "created": c.created,
                        "updated": c.updated,
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
            format!("Failed to list contexts: {e}"),
            None,
        )),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetContextInput {
    /// The context UUID.
    #[schemars(description = "UUID of the context to retrieve")]
    pub id: Uuid,
}

pub async fn get_context(
    svc: &TemperMcpService,
    input: GetContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    match temper_api::services::context_service::get_visible(pool, profile.id, input.id).await {
        Ok(c) => {
            let result = serde_json::json!({
                "id": c.id,
                "name": c.name,
                "owner_table": c.kb_owner_table,
                "owner_id": c.kb_owner_id,
                "created": c.created,
                "updated": c.updated,
            });
            let text = serde_json::to_string_pretty(&result)
                .unwrap_or_else(|_| "{}".to_string());
            Ok(CallToolResult::success(vec![
                rmcp::model::Content::text(text),
            ]))
        }
        Err(e) => Err(rmcp::ErrorData::internal_error(
            format!("Failed to get context: {e}"),
            None,
        )),
    }
}
