//! Resource tools — list, get, and create resources in the knowledge base.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::resource::{
    ContentResponse, ResourceCreateRequest, ResourceListParams, ResourceUpdateRequest,
};

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

/// MCP input for update_resource — bundles the resource ID (which REST takes
/// as a path parameter) together with the shared update payload.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceInput {
    /// UUID of the resource to update.
    pub id: Uuid,
    /// Fields to update.
    #[serde(flatten)]
    pub update: ResourceUpdateRequest,
}

/// MCP input for delete_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteResourceInput {
    /// UUID of the resource to delete.
    pub id: Uuid,
}

/// MCP input for get_resource_content.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetResourceContentInput {
    /// UUID of the resource whose content to retrieve.
    pub id: Uuid,
}

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

pub async fn list_resources(
    svc: &TemperMcpService,
    input: ResourceListParams,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let rows = temper_api::services::resource_service::list_visible(
        &svc.api_state.pool,
        profile.id,
        input,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to list resources: {e}"), None))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&rows),
    )]))
}

pub async fn get_resource(
    svc: &TemperMcpService,
    input: GetResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let row = temper_api::services::resource_service::get_visible(pool, profile.id, input.id)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
        })?;

    if input.include_content.unwrap_or(false) {
        let markdown =
            temper_api::services::resource_service::get_content(pool, profile.id, input.id)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Failed to get content: {e}"), None)
                })?;

        let response = ContentResponse {
            resource_id: row.id,
            markdown,
        };
        Ok(CallToolResult::success(vec![
            rmcp::model::Content::text(to_text(&row)),
            rmcp::model::Content::text(to_text(&response)),
        ]))
    } else {
        Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            to_text(&row),
        )]))
    }
}

pub async fn create_resource(
    svc: &TemperMcpService,
    input: ResourceCreateRequest,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let row =
        temper_api::services::resource_service::create(&svc.api_state.pool, profile.id, input)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to create resource: {e}"), None)
            })?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&row),
    )]))
}

pub async fn update_resource(
    svc: &TemperMcpService,
    input: UpdateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let row = temper_api::services::resource_service::update(
        &svc.api_state.pool,
        profile.id,
        input.id,
        input.update,
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to update resource: {e}"), None)
    })?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&row),
    )]))
}

pub async fn delete_resource(
    svc: &TemperMcpService,
    input: DeleteResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    temper_api::services::resource_service::delete(&svc.api_state.pool, profile.id, input.id)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to delete resource: {e}"), None)
        })?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&serde_json::json!({ "deleted": true, "id": input.id })),
    )]))
}

pub async fn get_resource_content(
    svc: &TemperMcpService,
    input: GetResourceContentInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let markdown = temper_api::services::resource_service::get_content(pool, profile.id, input.id)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to get content: {e}"), None)
        })?;

    let response = ContentResponse {
        resource_id: input.id,
        markdown,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}
