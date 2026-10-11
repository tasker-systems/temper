//! Context tools — list and inspect knowledge base contexts.
//!
//! # Execution crosses the network door (beat G3d — the fifth family to cross)
//!
//! Every tool forwards to its deployed `/api/contexts` route through a per-request
//! temper-client relay built from the request's `Parts`: Level 1 + 2 run at the API on the
//! caller's own bearer. The create path's owner resolution rides the same wire body
//! (`ContextCreateRequest.owner`) the route resolves server-side.
//!
//! # Parity deltas declared at the swap (the G3c delta format)
//!
//! - **NotFound / Conflict / 400 sentences arrive bare** — the direct mappers prefixed
//!   every one `{context}: `; the door carries the server's own sentence, the
//!   `Conflict: `/`Bad request: ` status labels stripped by the shared cause helper,
//!   kind and gate identical. The `403` requirement sentences are the TOOL's words
//!   (the wire's bare 403 carries none) and are byte-stable, their pins carried.
//! - **`get` of a retired-but-administered context now answers the row** — the
//!   route's restore-reachability fallback (`get_retired_administered`) sits behind
//!   the door, where the direct binding's `get_visible`-only read refused. Declared,
//!   not smuggled: the caller's own administered data, identical to what the CLI's
//!   `GET /api/contexts/{id}` already returns — MCP converges to the route family's
//!   own semantics. The suite pins no face on this read either way.
//! - **`create`'s owner-resolution `403`** (a caller who does not manage the team
//!   that would own the context) renders the tool's requirement sentence under
//!   `invalid_params`, where the direct binding mapped it to the internal
//!   catch-all — the declared caller-actionable delta.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_client::error::ClientError;
use temper_core::types::cognitive_maps::{ContextAnalyticsInput, ContextShapeInput};
use temper_core::types::context::{
    ContextCreateRequest, ReassignContextRequest, RenameContextRequest, ShareContextRequest,
};

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};
use crate::tools::cognitive_maps::{context_analytics, context_region_metrics, context_shape};

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

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

/// Map a context-write error onto an MCP error. The `403` (the authorization gate named by
/// `rule`), `404` (missing context or team), `409` (a taken slug) and `400` (an unusable name)
/// become invalid-params so the agent sees an actionable message rather than an opaque internal
/// error.
///
/// `Forbidden` and `NotFound` must stay **distinguishable** here: the gate answers `403` to a
/// caller who reads the context but does not administer it and `404` to one who cannot see it, and
/// collapsing the two into indistinguishable text would discard the disclosure distinction the
/// service deliberately draws.
///
/// The `403`'s requirement clause is the TOOL's sentence (the wire's bare 403 body carries no
/// requirement text), byte-identical to the direct binding's. The `404`/`409`/`400` sentences are
/// the server's own, carried through the door — the direct binding's `{context}: ` prefix does not
/// re-apply (the declared delta); 400/409 carry the API's rendered `Bad request: `/`Conflict: `
/// labels, stripped by the shared cause helper.
fn map_api_error(context: &str, rule: ForbiddenRule, err: ClientError) -> rmcp::ErrorData {
    match err {
        ClientError::Forbidden => rmcp::ErrorData::invalid_params(
            format!("{context} requires that {}", rule.requirement()),
            None,
        ),
        // A refusal that names why — a machine refused share, unshare or transfer, which take a
        // person — carries the server's own sentence, never an internal fault.
        ClientError::ForbiddenDetail { message } => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{context}: {message}"),
            None,
        ),
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::Conflict { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
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

/// The create path's own mapper: same voice as the writes, plus the owner-resolution
/// faces that ride the wire body. The bare `403` (a caller who does not manage the team
/// that would own the context) speaks the tool's requirement sentence — the direct
/// binding mapped this face to the internal catch-all, the declared delta.
fn map_create_err(err: ClientError) -> rmcp::ErrorData {
    match err {
        ClientError::Forbidden => rmcp::ErrorData::invalid_params(
            "create_context requires that you manage the team that will own it (owner/maintainer)"
                .to_string(),
            None,
        ),
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::Conflict { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        other => rmcp::ErrorData::internal_error(format!("create_context failed: {other}"), None),
    }
}

pub async fn list_contexts(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let rows = svc
        .relay_client(parts)?
        .contexts()
        .list()
        .await
        .across_auth(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to list contexts: {e}"), None)
        })?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&rows)),
    ]))
}

pub async fn get_context(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: GetContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    // A read denies with the server's own sentence (the same arm-for-arm shape the
    // orientation reads use); everything else stays the direct wrapper's fault voice.
    let row = svc
        .relay_client(parts)?
        .contexts()
        .get(input.id)
        .await
        .across_auth(|e| match e {
            ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
            other => {
                rmcp::ErrorData::internal_error(format!("Failed to get context: {other}"), None)
            }
        })?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&row)),
    ]))
}

pub async fn create_context(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ContextCreateRequest,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let row = svc
        .relay_client(parts)?
        .contexts()
        .create(&input.name, input.owner)
        .await
        .across_auth(map_create_err)?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&row)),
    ]))
}

/// Share a context into a team's read-reach, through the door. Authorized by the two-sided
/// `can_share` gate at the API (system-admin, OR the caller administers the context AND manages
/// the target team) before the write. Idempotent — `shared: false` when the share already existed.
pub async fn share_context(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ShareContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let outcome = svc
        .relay_client(parts)?
        .contexts()
        .share_team(
            input.context,
            &ShareContextRequest {
                team_id: input.team,
            },
        )
        .await
        .across_auth(|e| map_api_error("share_context", ForbiddenRule::ContextAndTargetTeam, e))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&outcome)),
    ]))
}

/// Transfer a context's ownership to a team, through the door. Authorized by the two-sided
/// `can_share` gate at the API. Binding a context to a team is the single path to shared
/// authorship — read-sharing stays [`share_context`]; writing into a context requires team
/// ownership. Idempotent — `reassigned: false` when the context was already owned by the target
/// team.
pub async fn transfer_context(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: TransferContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let outcome = svc
        .relay_client(parts)?
        .contexts()
        .reassign(
            input.context,
            &ReassignContextRequest {
                to_team_id: input.to_team,
            },
        )
        .await
        .across_auth(|e| {
            map_api_error("transfer_context", ForbiddenRule::ContextAndTargetTeam, e)
        })?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&outcome)),
    ]))
}

/// Unshare a context from a team, through the door. Same `can_share` authorization as
/// [`share_context`], at the API. No-op safe — `unshared: false` when there was no share to
/// remove.
pub async fn unshare_context(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ShareContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let outcome = svc
        .relay_client(parts)?
        .contexts()
        .unshare_team(input.context, input.team)
        .await
        .across_auth(|e| {
            map_api_error("unshare_context", ForbiddenRule::ContextAndTargetTeam, e)
        })?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&outcome)),
    ]))
}

/// Rename a context — the one act that moves its `(name, slug)` identity pair in place.
/// Authorized by the one-sided `ContextAdminAuthority` gate at the API (you administer the
/// context, or you are an instance admin); this tool adds no authorization of its own. The slug is
/// **derived** from the name, so a rename **re-addresses** the context — the outcome carries the
/// composed `context_ref` to use from now on. Idempotent — `renamed: false` when the canonical
/// name already equalled the stored one.
pub async fn rename_context(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: RenameContextInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let outcome = svc
        .relay_client(parts)?
        .contexts()
        .rename(input.context, &RenameContextRequest { name: input.name })
        .await
        .across_auth(|e| {
            map_api_error("rename_context", ForbiddenRule::ContextAdministration, e)
        })?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&outcome)),
    ]))
}

// ── Consolidated read tool (5→1) ───────────────────────────────────────────────

/// The context-read view to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum ContextReadView {
    /// List all contexts available to the caller.
    List,
    /// Get details of a specific context by UUID.
    Get,
    /// Read the context's materialized regions (most salient first).
    Shape,
    /// Read per-region analytics metrics for the context.
    Metrics,
    /// Read the context's staleness readout: when its shape was last materialized, the latest
    /// readable touch to its regions/edges, and whether the read is stale.
    Analytics,
}

/// Consolidated context-read tool — one read tool with a `view` discriminator.
///
/// Collapses `list_contexts`, `get_context`, `context_shape`, `context_region_metrics` and
/// `context_analytics` into a single MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ContextReadInput {
    /// Which context read to perform.
    pub view: ContextReadView,
    /// Context UUID. Required for `get`; ignored for `list`.
    #[serde(default)]
    pub id: Option<Uuid>,
    /// Context ref (`@me/<slug>`, `+<team>/<slug>`, or UUID). Required for `shape`, `metrics` and `analytics`; ignored for `list` and `get`.
    #[serde(default)]
    pub context: Option<String>,
    /// Optional lens ref to filter regions. Used with `shape` and `metrics`.
    #[serde(default)]
    pub lens: Option<String>,
}

/// Dispatch the consolidated context-read tool.
pub async fn context_read(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ContextReadInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.view {
        ContextReadView::List => list_contexts(svc, parts).await,
        ContextReadView::Get => {
            let id = input.id.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("get requires `id`".to_string(), None)
            })?;
            get_context(svc, parts, GetContextInput { id }).await
        }
        ContextReadView::Shape => {
            let context = input.context.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("shape requires `context`".to_string(), None)
            })?;
            context_shape(
                svc,
                parts,
                ContextShapeInput {
                    context,
                    lens: input.lens,
                },
            )
            .await
        }
        ContextReadView::Metrics => {
            let context = input.context.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("metrics requires `context`".to_string(), None)
            })?;
            context_region_metrics(
                svc,
                parts,
                ContextShapeInput {
                    context,
                    lens: input.lens,
                },
            )
            .await
        }
        ContextReadView::Analytics => {
            let context = input.context.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("analytics requires `context`".to_string(), None)
            })?;
            context_analytics(svc, parts, ContextAnalyticsInput { context }).await
        }
    }
}

// ── Consolidated write tool (5→1) ─────────────────────────────────────────────

/// The context-manage action to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum ContextManageAction {
    /// Create a new context.
    Create,
    /// Rename a context (re-addresses it — the old slug stops resolving).
    Rename,
    /// Share a context into a team's read-reach.
    Share,
    /// Unshare a context from a team.
    Unshare,
    /// Transfer a context's ownership to a team.
    Transfer,
}

/// Consolidated context-manage tool — one write tool with an `action` discriminator.
///
/// Collapses `create_context`, `rename_context`, `share_context`, `unshare_context`,
/// and `transfer_context` into a single MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ContextManageInput {
    /// Which context action to perform.
    pub action: ContextManageAction,
    /// The display name for the new context. Required for `create`.
    #[serde(default)]
    pub name: Option<String>,
    /// Who owns the new context. Used with `create`.
    #[serde(default)]
    pub owner: Option<temper_core::context_ref::ContextOwnerRef>,
    /// Context UUID. Required for `rename`, `share`, `unshare`, `transfer`; ignored for `create`.
    #[serde(default)]
    pub context: Option<Uuid>,
    /// Team UUID. Required for `share`, `unshare`, `transfer`.
    #[serde(default)]
    pub team: Option<Uuid>,
}

/// Dispatch the consolidated context-manage tool.
pub async fn context_manage(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ContextManageInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.action {
        ContextManageAction::Create => {
            let name = input.name.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("create requires `name`".to_string(), None)
            })?;
            create_context(
                svc,
                parts,
                ContextCreateRequest {
                    name,
                    owner: input.owner,
                },
            )
            .await
        }
        ContextManageAction::Rename => {
            let context = input.context.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("rename requires `context`".to_string(), None)
            })?;
            let name = input.name.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("rename requires `name`".to_string(), None)
            })?;
            rename_context(svc, parts, RenameContextInput { context, name }).await
        }
        ContextManageAction::Share => {
            let context = input.context.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("share requires `context`".to_string(), None)
            })?;
            let team = input.team.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("share requires `team`".to_string(), None)
            })?;
            share_context(svc, parts, ShareContextInput { context, team }).await
        }
        ContextManageAction::Unshare => {
            let context = input.context.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("unshare requires `context`".to_string(), None)
            })?;
            let team = input.team.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("unshare requires `team`".to_string(), None)
            })?;
            unshare_context(svc, parts, ShareContextInput { context, team }).await
        }
        ContextManageAction::Transfer => {
            let context = input.context.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("transfer requires `context`".to_string(), None)
            })?;
            let to_team = input.team.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("transfer requires `team`".to_string(), None)
            })?;
            transfer_context(svc, parts, TransferContextInput { context, to_team }).await
        }
    }
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
            ClientError::Forbidden,
        );
        assert!(
            forbidden.message.contains("administer the context"),
            "{forbidden:?}"
        );
        let not_found = map_api_error(
            "share_context",
            ForbiddenRule::ContextAndTargetTeam,
            ClientError::NotFound {
                message: "team seed-team not found or not readable".to_string(),
            },
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
    fn map_api_error_carries_a_detailed_refusal_as_the_callers_not_a_fault() {
        let refused = map_api_error(
            "share_context",
            ForbiddenRule::ContextAndTargetTeam,
            ClientError::ForbiddenDetail {
                message: "this action is not available to a machine principal".to_string(),
            },
        );
        assert_eq!(
            refused.code,
            rmcp::model::ErrorCode::INVALID_REQUEST,
            "{refused:?}"
        );
        assert_eq!(
            refused.message,
            "share_context: this action is not available to a machine principal"
        );
    }

    #[test]
    fn rename_forbidden_does_not_render_the_share_requirement() {
        // Rename is gated one-sided (`ContextAdminAuthority`); there is no target team in it.
        let forbidden = map_api_error(
            "rename_context",
            ForbiddenRule::ContextAdministration,
            ClientError::Forbidden,
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
        // The door's 409 body carries the API's rendered Display (`Conflict: …`); the
        // mapper strips the status label and speaks the server's own sentence bare.
        let conflict = map_api_error(
            "rename_context",
            ForbiddenRule::ContextAdministration,
            ClientError::Conflict {
                message:
                    "Conflict: @cole already owns a context with slug 'notes'; pick another name"
                        .to_string(),
            },
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
        assert!(
            !conflict.message.contains("Conflict:"),
            "the status label is stripped — the sentence speaks bare: {conflict:?}"
        );
    }

    #[test]
    fn map_api_error_renders_bad_request_as_invalid_params_carrying_the_message() {
        // Same story for the 400: the `Bad request: ` label comes off at the mapper.
        let bad_request = map_api_error(
            "rename_context",
            ForbiddenRule::ContextAdministration,
            ClientError::Server {
                status: 400,
                message: "Bad request: name '!!!' has no addressable content".to_string(),
            },
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
        assert!(
            !bad_request.message.contains("Bad request:"),
            "the status label is stripped: {bad_request:?}"
        );
    }
}
