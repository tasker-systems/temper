//! Search tool — vector similarity search across resources.

use rmcp::model::CallToolResult;

use temper_core::types::api::SearchParams;

use crate::service::TemperMcpService;

pub async fn search(
    svc: &TemperMcpService,
    input: SearchParams,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let rows = temper_api::services::search_service::search(&svc.api_state.pool, profile.id, input)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Search failed: {e}"), None))?;

    let text =
        serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![
        rmcp::model::Content::text(text),
    ]))
}
