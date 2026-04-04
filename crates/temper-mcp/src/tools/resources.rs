//! Resource tools — list, get, and create resources in the knowledge base.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::resource::{ContentResponse, ResourceCreateRequest, ResourceListParams};

use crate::service::TemperMcpService;

/// MCP input for get_resource — extends the core ID lookup with an
/// `include_content` flag specific to the MCP tool interface.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetResourceInput {
    /// UUID of the resource to retrieve.
    pub id: Uuid,
    /// If true, includes the full markdown content of the resource.
    pub include_content: Option<bool>,
}

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

pub async fn list_resources(
    svc: &TemperMcpService,
    input: ResourceListParams,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let rows =
        temper_api::services::resource_service::list_visible(&svc.api_state.pool, profile.id, input)
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to list resources: {e}"), None))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::Content::text(to_text(&rows)),
    ]))
}

pub async fn get_resource(
    svc: &TemperMcpService,
    input: GetResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let row = temper_api::services::resource_service::get_visible(pool, profile.id, input.id)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None))?;

    if input.include_content.unwrap_or(false) {
        let markdown =
            temper_api::services::resource_service::get_content(pool, profile.id, input.id)
                .await
                .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get content: {e}"), None))?;

        let response = ContentResponse {
            resource_id: row.id,
            markdown,
        };
        Ok(CallToolResult::success(vec![
            rmcp::model::Content::text(to_text(&row)),
            rmcp::model::Content::text(to_text(&response)),
        ]))
    } else {
        Ok(CallToolResult::success(vec![
            rmcp::model::Content::text(to_text(&row)),
        ]))
    }
}

pub async fn create_resource(
    svc: &TemperMcpService,
    input: ResourceCreateRequest,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let row = temper_api::services::resource_service::create(&svc.api_state.pool, profile.id, input)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to create resource: {e}"), None))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::Content::text(to_text(&row)),
    ]))
}
