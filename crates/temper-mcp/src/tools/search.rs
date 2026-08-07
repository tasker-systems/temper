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

    // ONE content block carrying the whole response — both arms, their dispositions, and the shared
    // scope. Deliberately not two blocks of hits: splitting them across content blocks would make an
    // agent reassemble the pair, and reassembly is exactly where a reader re-invents the merge this
    // shape exists to prevent. The per-arm `reason`/`hint` ride inside, so the old "append a second
    // block when the scope stage has something to say" branch has nothing left to do.
    let body = serde_json::to_string_pretty(&response)
        .unwrap_or_else(|_| "{\"exact\":{},\"wide\":{}}".to_string());
    let contents = vec![rmcp::model::Content::text(body)];
    Ok(CallToolResult::success(contents))
}
