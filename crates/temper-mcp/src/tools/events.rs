//! Event tools — list knowledge base events for debugging and auditing.

use rmcp::model::CallToolResult;

use temper_core::types::api::EventListParams;

use crate::service::TemperMcpService;

pub async fn list_events(
    svc: &TemperMcpService,
    input: EventListParams,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let rows =
        temper_api::services::event_service::list_visible(&svc.api_state.pool, profile.id, input)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to list events: {e}"), None)
            })?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}
