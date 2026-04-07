//! Doc type tools — list available document types.

use rmcp::model::CallToolResult;

use crate::service::TemperMcpService;

pub async fn list_doc_types(svc: &TemperMcpService) -> Result<CallToolResult, rmcp::ErrorData> {
    let _profile = svc.require_profile().await?;

    let rows = temper_api::services::doc_type_service::list_all(&svc.api_state.pool)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to list doc types: {e}"), None)
        })?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}
