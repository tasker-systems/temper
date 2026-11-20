//! Search tool — full-text and/or vector similarity search across resources.
//!
//! Execution crosses the DEPLOYED API over the wire (beat G3b, the network door — the
//! second family to cross after resources): the handler drives a per-request
//! temper-client HTTP relay built from the request's `Parts` and never touches the
//! pool or a service function directly. The tool input IS `SearchParams`, the same
//! wire type `/api/search` deserializes, so nothing is restated or re-shaped here —
//! the one content block below is the only MCP-local shaping.

use rmcp::model::CallToolResult;

use temper_client::error::ClientError;
use temper_core::types::api::SearchParams;

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};

pub async fn search(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: SearchParams,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;

    let response = client
        .search()
        .search_with_params(&input)
        .await
        .across_auth(|e| map_search_error("search", e))?;

    // ONE content block carrying the whole response — both arms, their dispositions, and the shared
    // scope. Deliberately not two blocks of hits: splitting them across content blocks would make an
    // agent reassemble the pair, and reassembly is exactly where a reader re-invents the merge this
    // shape exists to prevent. The per-arm `reason`/`hint` ride inside, so the old "append a second
    // block when the scope stage has something to say" branch has nothing left to do.
    let body = serde_json::to_string_pretty(&response)
        .unwrap_or_else(|_| "{\"exact\":{},\"wide\":{}}".to_string());
    let contents = vec![rmcp::model::ContentBlock::text(body)];
    Ok(CallToolResult::success(contents))
}

/// Map a search-path error onto an MCP error.
///
/// A 400 is a CALLER error the caller can act on — the degenerate-embedding face
/// (`reject_degenerate_embedding` refuses a zero-norm vector as `ApiError::BadRequest`
/// at the API) — so it speaks the server's own sentence as `invalid_params`, the same
/// care `contexts.rs::map_api_error` gives `BadRequest`. The direct binding wrapped
/// every error as `internal_error`; this one face's rendered kind changed with the
/// door, and the parity suite pins the change as its declared delta. [The 400-label
/// sweep, 2026-09-26] the sentence now renders bare — `api_error_cause` strips the
/// API Display's `Bad request: ` status label the relayed body carries, and the
/// arm's own `search: ` context prefix drops with it (a label stacked on a label) —
/// the face the G3c families already speak, adopted as the family's second declared
/// parity delta. Kind and gate identical. Everything else stays opaque, matching the
/// established pattern.
fn map_search_error(context: &str, err: ClientError) -> rmcp::ErrorData {
    match err {
        ClientError::Server {
            status: 400,
            message,
        }
        | ClientError::BadRequest { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        other => rmcp::ErrorData::internal_error(format!("{context} failed: {other}"), None),
    }
}
