//! Context tools — list and inspect knowledge base contexts.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::context::ContextCreateRequest;

use crate::service::TemperMcpService;

/// MCP input for get_context.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetContextInput {
    /// UUID of the context to retrieve.
    pub id: Uuid,
}

pub async fn list_contexts(svc: &TemperMcpService) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let rows = temper_api::services::context_service::list_visible(&svc.api_state.pool, profile.id)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to list contexts: {e}"), None)
        })?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

pub async fn get_context(
    svc: &TemperMcpService,
    input: GetContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let row = temper_api::services::context_service::get_visible(
        &svc.api_state.pool,
        profile.id,
        input.id,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get context: {e}"), None))?;

    let text = serde_json::to_string_pretty(&row).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

pub async fn create_context(
    svc: &TemperMcpService,
    input: ContextCreateRequest,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let row =
        temper_api::services::context_service::create(&svc.api_state.pool, profile.id, &input.name)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to create context: {e}"), None)
            })?;

    let text = serde_json::to_string_pretty(&row).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}
