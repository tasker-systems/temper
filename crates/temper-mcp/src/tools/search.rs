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

    let response = temper_services::backend::substrate_read::search_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        input,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Search failed: {e}"), None))?;

    // First content block stays the ranked-results array — the unchanged tool-output contract.
    let rows_text =
        serde_json::to_string_pretty(&response.results).unwrap_or_else(|_| "[]".to_string());
    let mut contents = vec![rmcp::model::Content::text(rows_text)];

    // Additive (issue #360): when the scope stage has something to say (empty / out-of-scope /
    // degraded — i.e. a hint is present), append a second block carrying the structured
    // diagnostics, so an agent can branch on `reason`/`scope_size` instead of puzzling over an
    // empty array. On the happy path this block is absent and the output is byte-identical to before.
    if let Some(diag) = response.diagnostics.as_ref().filter(|d| d.hint.is_some()) {
        if let Ok(diag_text) = serde_json::to_string_pretty(diag) {
            contents.push(rmcp::model::Content::text(format!(
                "search diagnostics: {diag_text}"
            )));
        }
    }
    Ok(CallToolResult::success(contents))
}
