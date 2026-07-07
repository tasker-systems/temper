//! Context tools — list and inspect knowledge base contexts.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::context::{ContextCreateRequest, ShareContextRequest};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::error::ApiError;

use crate::service::TemperMcpService;

/// Map a context-service error onto an MCP error. `Forbidden` (the admin gate on
/// share/unshare) and `NotFound` (missing context or team) become invalid-params
/// so the agent sees an actionable message rather than an opaque internal error.
fn map_api_error(context: &str, err: ApiError) -> rmcp::ErrorData {
    match err {
        ApiError::Forbidden => {
            rmcp::ErrorData::invalid_params(format!("{context} requires system-admin"), None)
        }
        ApiError::NotFound => {
            rmcp::ErrorData::invalid_params(format!("{context}: context or team not found"), None)
        }
        other => rmcp::ErrorData::internal_error(format!("{context} failed: {other}"), None),
    }
}

/// MCP input for get_context.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetContextInput {
    /// UUID of the context to retrieve.
    pub id: Uuid,
}

/// MCP input for share_context / unshare_context: a context and a team, both by UUID
/// (get them from `list_contexts` / your team listing).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShareContextInput {
    /// UUID of the context to share/unshare.
    pub context: Uuid,
    /// UUID of the team to share into / unshare from.
    pub team: Uuid,
}

pub async fn list_contexts(svc: &TemperMcpService) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let rows = temper_services::services::context_service::list_visible(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to list contexts: {e}"), None))?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

pub async fn get_context(
    svc: &TemperMcpService,
    input: GetContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let row = temper_services::services::context_service::get_visible(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        ContextId::from(input.id),
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get context: {e}"), None))?;

    let text = serde_json::to_string_pretty(&row).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

pub async fn create_context(
    svc: &TemperMcpService,
    input: ContextCreateRequest,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let caller = ProfileId::from(profile.id);

    let (owner_table, owner_id) = temper_services::services::context_service::resolve_create_owner(
        &svc.api_state.pool,
        caller,
        input.owner.as_ref(),
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to resolve owner: {e}"), None))?;

    let row = temper_services::services::context_service::create(
        &svc.api_state.pool,
        &owner_table,
        owner_id,
        &input.name,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to create context: {e}"), None))?;

    let text = serde_json::to_string_pretty(&row).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

/// Share a context into a team's read-reach. SERVICE-DIRECT, admin-gated (see
/// `context_service::share`, which enforces `is_system_admin` before the write).
/// Idempotent — `shared: false` when the share already existed.
pub async fn share_context(
    svc: &TemperMcpService,
    input: ShareContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let outcome = temper_services::services::context_service::share(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        input.context,
        &ShareContextRequest {
            team_id: input.team,
        },
    )
    .await
    .map_err(|e| map_api_error("share_context", e))?;

    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

/// Unshare a context from a team. SERVICE-DIRECT, admin-gated (see [`share_context`]).
/// No-op safe — `unshared: false` when there was no share to remove.
pub async fn unshare_context(
    svc: &TemperMcpService,
    input: ShareContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let outcome = temper_services::services::context_service::unshare(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        input.context,
        input.team,
    )
    .await
    .map_err(|e| map_api_error("unshare_context", e))?;

    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_context_input_deserializes() {
        let ctx = Uuid::now_v7();
        let team = Uuid::now_v7();
        let json = format!(r#"{{"context":"{ctx}","team":"{team}"}}"#);
        let input: ShareContextInput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(input.context, ctx);
        assert_eq!(input.team, team);
    }

    #[test]
    fn map_api_error_distinguishes_forbidden_and_notfound() {
        // Forbidden (admin gate) and NotFound both surface as actionable
        // invalid-params, not opaque internal errors.
        let forbidden = map_api_error("share_context", ApiError::Forbidden);
        assert!(forbidden.message.contains("system-admin"), "{forbidden:?}");
        let not_found = map_api_error("share_context", ApiError::NotFound);
        assert!(not_found.message.contains("not found"), "{not_found:?}");
    }
}
