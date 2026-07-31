//! Context tools — list and inspect knowledge base contexts.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::context::{ContextCreateRequest, ShareContextRequest};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::error::ApiError;

use crate::service::TemperMcpService;

/// Which admission rule a context tool's `403` describes.
///
/// The context gates genuinely differ, so one `Forbidden` message cannot be correct for all of
/// them: share / unshare / transfer are gated two-sided (`can_share` — administer the context AND
/// manage the target team), while rename is gated one-sided (`ContextAdminAuthority` — administer
/// the context, i.e. own it or manage the owning team). A key rather than a match on the `context`
/// label, so the rendering never depends on a display string.
#[derive(Debug, Clone, Copy)]
enum ForbiddenRule {
    /// The two-sided `can_share` gate behind share / unshare / transfer.
    ContextAndTargetTeam,
    /// The one-sided `ContextAdminAuthority` gate behind rename.
    ContextAdministration,
}

impl ForbiddenRule {
    /// The requirement clause, rendered after `"<tool> requires that "`.
    fn requirement(self) -> &'static str {
        match self {
            Self::ContextAndTargetTeam => {
                "you administer the context and manage the target team (owner/maintainer), \
                 or that you are an instance administrator"
            }
            Self::ContextAdministration => {
                "you administer the context — own it, or manage the owning team \
                 (owner/maintainer) — or that you are an instance administrator"
            }
        }
    }
}

/// Map a context-service error onto an MCP error. `Forbidden` (the authorization gate named by
/// `rule`), `NotFound` (missing context or team), `Conflict` (a taken slug) and `BadRequest` (an
/// unusable name) become invalid-params so the agent sees an actionable message rather than an
/// opaque internal error.
///
/// `Forbidden` and `NotFound` must stay **distinguishable** here: the gate answers `403` to a
/// caller who reads the context but does not administer it and `404` to one who cannot see it, and
/// collapsing the two into indistinguishable text would discard the disclosure distinction the
/// service deliberately draws.
fn map_api_error(context: &str, rule: ForbiddenRule, err: ApiError) -> rmcp::ErrorData {
    match err {
        ApiError::Forbidden => rmcp::ErrorData::invalid_params(
            format!("{context} requires that {}", rule.requirement()),
            None,
        ),
        // Carry the service's own message rather than replacing it with a constant: it names
        // which of the context or the team was unresolvable, which this arm could only guess at.
        ApiError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("{context}: {msg}"), None)
        }
        // Same reason, and the actionable half in both cases is exactly what the service put in
        // the message: the `409` names the colliding slug, the `400` names what about the given
        // name has no addressable content. A constant here could only guess at either.
        ApiError::Conflict(msg) | ApiError::BadRequest(msg) => {
            rmcp::ErrorData::invalid_params(format!("{context}: {msg}"), None)
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

/// MCP input for transfer_context: a context and the team that will own it, both by UUID.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TransferContextInput {
    /// UUID of the context to transfer.
    pub context: Uuid,
    /// UUID of the team the context will be owned by.
    pub to_team: Uuid,
}

/// MCP input for rename_context: a context by UUID and the new display name it should carry.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenameContextInput {
    /// UUID of the context to rename.
    pub context: Uuid,
    /// The new display name. The addressable slug is derived from it server-side — there is
    /// deliberately no independent slug parameter — so a rename re-addresses the context.
    pub name: String,
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

/// Share a context into a team's read-reach. SERVICE-DIRECT, authorized by
/// `context_service::share` (its two-sided `can_share` gate: system-admin, OR the caller
/// administers the context AND manages the target team) before the write.
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
    .map_err(|e| map_api_error("share_context", ForbiddenRule::ContextAndTargetTeam, e))?;

    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

/// Transfer a context's ownership to a team. SERVICE-DIRECT, authorized by
/// `context_service::reassign` (the two-sided `can_share` gate) before the write. Binding a
/// context to a team is the single path to shared authorship — read-sharing stays
/// [`share_context`]; writing into a context requires team ownership. Idempotent —
/// `reassigned: false` when the context was already owned by the target team.
pub async fn transfer_context(
    svc: &TemperMcpService,
    input: TransferContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let outcome = temper_services::services::context_service::reassign(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        input.context,
        input.to_team,
    )
    .await
    .map_err(|e| map_api_error("transfer_context", ForbiddenRule::ContextAndTargetTeam, e))?;

    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

/// Unshare a context from a team. SERVICE-DIRECT, same `can_share` authorization as
/// [`share_context`]. No-op safe — `unshared: false` when there was no share to remove.
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
    .map_err(|e| map_api_error("unshare_context", ForbiddenRule::ContextAndTargetTeam, e))?;

    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

/// Rename a context — the one act that moves its `(name, slug)` identity pair in place.
/// SERVICE-DIRECT, authorized by `context_service::rename` (the one-sided `ContextAdminAuthority`
/// gate: you administer the context, or you are an instance admin) before the write; this tool adds
/// no authorization of its own. The slug is **derived** from the name, so a rename **re-addresses**
/// the context — the outcome carries the composed `context_ref` to use from now on. Idempotent —
/// `renamed: false` when the canonical name already equalled the stored one.
pub async fn rename_context(
    svc: &TemperMcpService,
    input: RenameContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let outcome = temper_services::services::context_service::rename(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        input.context,
        &input.name,
    )
    .await
    .map_err(|e| map_api_error("rename_context", ForbiddenRule::ContextAdministration, e))?;

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
    fn rename_context_input_deserializes() {
        let ctx = Uuid::now_v7();
        let json = format!(r#"{{"context":"{ctx}","name":"Temper KB"}}"#);
        let input: RenameContextInput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(input.context, ctx);
        assert_eq!(input.name, "Temper KB");
    }

    #[test]
    fn map_api_error_distinguishes_forbidden_and_notfound() {
        // Forbidden (the share authorization gate) and NotFound both surface as
        // actionable invalid-params, not opaque internal errors.
        let forbidden = map_api_error(
            "share_context",
            ForbiddenRule::ContextAndTargetTeam,
            ApiError::Forbidden,
        );
        assert!(
            forbidden.message.contains("administer the context"),
            "{forbidden:?}"
        );
        let not_found = map_api_error(
            "share_context",
            ForbiddenRule::ContextAndTargetTeam,
            ApiError::NotFound("team seed-team not found or not readable".to_string()),
        );
        assert!(
            not_found.message.contains("seed-team"),
            "the service's message must survive onto MCP, not be replaced: {not_found:?}"
        );
        assert_ne!(
            forbidden.message, not_found.message,
            "403 and 404 must stay distinguishable at this surface"
        );
    }

    #[test]
    fn rename_forbidden_does_not_render_the_share_requirement() {
        // Rename is gated one-sided (`ContextAdminAuthority`); there is no target team in it.
        let forbidden = map_api_error(
            "rename_context",
            ForbiddenRule::ContextAdministration,
            ApiError::Forbidden,
        );
        assert!(
            forbidden.message.contains("administer the context"),
            "{forbidden:?}"
        );
        assert!(
            !forbidden.message.contains("target team"),
            "rename must not render the share/transfer two-sided requirement: {forbidden:?}"
        );
    }

    #[test]
    fn map_api_error_renders_conflict_as_invalid_params_carrying_the_message() {
        let conflict = map_api_error(
            "rename_context",
            ForbiddenRule::ContextAdministration,
            ApiError::Conflict(
                "@cole already owns a context with slug 'notes'; pick another name".to_string(),
            ),
        );
        assert_eq!(
            conflict.code,
            rmcp::model::ErrorCode::INVALID_PARAMS,
            "a taken slug is caller-fixable, not an internal error: {conflict:?}"
        );
        assert!(
            conflict.message.contains("slug 'notes'"),
            "the colliding slug is the actionable half and must survive: {conflict:?}"
        );
    }

    #[test]
    fn map_api_error_renders_bad_request_as_invalid_params_carrying_the_message() {
        let bad_request = map_api_error(
            "rename_context",
            ForbiddenRule::ContextAdministration,
            ApiError::BadRequest("name '!!!' has no addressable content".to_string()),
        );
        assert_eq!(
            bad_request.code,
            rmcp::model::ErrorCode::INVALID_PARAMS,
            "an unusable name is a caller-fixable parameter problem: {bad_request:?}"
        );
        assert!(
            bad_request.message.contains("no addressable content"),
            "the service's message must survive onto MCP: {bad_request:?}"
        );
    }
}
