//! Profile tools — retrieve the authenticated user's profile.

use rmcp::model::CallToolResult;

use crate::service::TemperMcpService;

pub async fn get_profile(svc: &TemperMcpService) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let text = serde_json::to_string_pretty(&profile).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}
