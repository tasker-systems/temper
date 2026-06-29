//! Search tool — full-text and/or vector similarity search across resources.

use rmcp::model::CallToolResult;

use temper_core::types::api::SearchParams;
use temper_core::types::ids::ProfileId;

use crate::service::TemperMcpService;

pub async fn search(
    svc: &TemperMcpService,
    input: SearchParams,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let rows = temper_services::backend::substrate_read::search_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        input,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Search failed: {e}"), None))?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}
