//! Agent-invocation envelope tools — open, close, show, and list.
//!
//! # Execution crosses the network door (beat G3d — the fifth family to cross)
//!
//! Every tool forwards to its deployed `/api/invocations` route through a per-request
//! temper-client relay built from the request's `Parts` — the same routes the CLI calls and
//! the G3c act-envelope ride already exercises. Level 1 + 2 run at the API on the caller's
//! own bearer; `Surface::Mcp` no longer rides the command — it rides the door's planted
//! carrier (`X-Temper-Relayed-Surface: mcp`), which is what the ledger's `<handle>@mcp`
//! emitter attribution watches.
//!
//! # Parity deltas declared at the swap (the G3c delta format)
//!
//! - **THE closed-invocation 409** — the direct `map_err` had no Conflict arm and
//!   rendered the terminal-transition refusal `internal_error`; the wire's 409 is
//!   caller-actionable, so it now renders `invalid_params` with the server's own
//!   sentence (the G3c flipped delta's twin; the suite pin flips in the same commit).
//! - **NotFound prefixes drop** — the direct mapper prefixed every not-found
//!   `{action}: `; the door carries the server's own sentence un-prefixed, kind and
//!   gate identical (the G3c delta; the suite's contains-based 404 pins carry green).
//! - **`show` of an unknown/unreadable envelope** — the direct readback answered the
//!   JSON text `null`; the wire route 404s (deny and absent indistinguishable, the
//!   leak-safe contract), so the door renders `invalid_params` with the route's
//!   sentence. The `list` read keeps its deny-with-data posture: an outsider's list is
//!   their own (empty) view, never an error.
//! - The `ForbiddenDetail` (detailed authorship) and terse `Forbidden` open-refusal
//!   arms KEEP their `{action}: ` prefixes — the tool's disclosure dialect, byte-stable.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use temper_client::error::ClientError;
use temper_core::types::invocation::{
    Disposition, InvocationCloseInput, InvocationListInput, InvocationOpenInput,
    InvocationShowInput,
};
use temper_core::types::invocation_requests::{
    CloseInvocationRequest, InvocationCloseAck, OpenInvocationRequest,
};

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

/// Map a door error onto an MCP protocol error, arm-for-arm with the direct binding.
///
/// **The `Forbidden` arm is reachable only from `invocation_open`**, which is why its text names the
/// cognitive map rather than the invocation. `invocation_close` puts its gate in the `WHERE` of the
/// lookup, so an unreadable or absent envelope collapses to `NotFound` there and it never produces a
/// `403` at all. The incumbent text — *"cannot access this invocation"* — was wrong on both counts:
/// `open` checks the **map**, and it does so against an invocation that does not yet exist. The
/// phrasing here is `steward.rs`'s, which already renders this same refusal.
///
/// `ForbiddenDetail` carries the gate's own sentence, which names the missing capability. It is a
/// distinct arm rather than a widened `Forbidden` so the terse refusal below stays byte-stable for
/// the caller who cannot read the map — see the wire's `FORBIDDEN_DETAIL` code for why that split is
/// the disclosure boundary and not a formatting choice.
fn map_err(e: ClientError, action: &str) -> rmcp::ErrorData {
    match e {
        // The server's own sentence, un-prefixed: the direct binding wrapped each not-found
        // with "{action}: ", a prefix the door does not re-apply — the kind (invalid_params)
        // and the gate are identical.
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::Server {
            status: 400,
            message,
        }
        | ClientError::BadRequest { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        // The door's 409 is caller-actionable — the direct catch-all rendered it
        // internal_error (the declared delta).
        ClientError::Conflict { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        // A refusal that named the capability it withheld — carry the gate's own sentence
        // under the tool's action prefix (the disclosure dialect, byte-stable).
        ClientError::ForbiddenDetail { message } => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: {message}"),
            None,
        ),
        ClientError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: cannot author this cognitive map"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

fn parse_cogmap(s: &str) -> Result<uuid::Uuid, rmcp::ErrorData> {
    temper_workflow::operations::parse_ref(s)
        .map(|p| p.uuid())
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))
}

fn parse_invocation(s: &str) -> Result<uuid::Uuid, rmcp::ErrorData> {
    Ok(temper_workflow::operations::parse_ref(s)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad invocation ref: {e}"), None))?
        .uuid())
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

pub async fn invocation_open(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: InvocationOpenInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let originating_cogmap = parse_cogmap(&input.originating_cogmap)?;
    let parent_cogmap = match input.parent_cogmap.as_deref() {
        Some(p) => Some(parse_cogmap(p)?),
        None => None,
    };

    // The wire request is the tool input 1:1 — the Surface origin the direct command
    // carried dies here; the door's planted carrier is what stamps `@mcp` at the ledger.
    let req = OpenInvocationRequest {
        trigger_kind: input.trigger_kind,
        originating_cogmap,
        parent_cogmap,
    };

    let ack = svc
        .relay_client(parts)?
        .invocations()
        .open(&req)
        .await
        .across_auth(|e| map_err(e, "invocation_open"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&ack)),
    ]))
}

pub async fn invocation_close(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: InvocationCloseInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let invocation = parse_invocation(&input.invocation)?;

    let disposition = input.disposition;
    let req = CloseInvocationRequest {
        disposition,
        outcome: input.outcome.unwrap_or(serde_json::Value::Null),
    };

    // The route answers 204 — the ack is composed from the request, exactly as the direct
    // binding composed it.
    svc.relay_client(parts)?
        .invocations()
        .close(invocation, &req)
        .await
        .across_auth(|e| map_err(e, "invocation_close"))?;

    let ack = InvocationCloseAck {
        invocation_id: invocation,
        disposition,
    };
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&ack)),
    ]))
}

/// The invocation-read mapper: a read's only caller-actionable arm is the route's
/// uniform 404 (deny and absent indistinguishable — the leak-safe contract, where the
/// direct readback answered deny-with-null: the declared delta); everything else stays
/// opaque under the read wrapper's own voice.
fn map_read_err(e: ClientError) -> rmcp::ErrorData {
    match e {
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        other => rmcp::ErrorData::internal_error(format!("invocation_show failed: {other}"), None),
    }
}

pub async fn invocation_show(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: InvocationShowInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let invocation = parse_invocation(&input.invocation)?;

    let view = svc
        .relay_client(parts)?
        .invocations()
        .show(invocation)
        .await
        .across_auth(map_read_err)?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&view)),
    ]))
}

pub async fn invocation_list(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: InvocationListInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let cogmap = match input.cogmap.as_deref() {
        Some(c) => Some(parse_invocation(c)?),
        None => None,
    };

    // Deny is DATA here, never an error: the route self-scopes to the caller's own
    // reach, so an outsider's list is their own (empty) view — the read posture the
    // direct binding held.
    let rows = svc
        .relay_client(parts)?
        .invocations()
        .list(cogmap, input.status)
        .await
        .across_auth(|e| {
            rmcp::ErrorData::internal_error(format!("invocation_list failed: {e}"), None)
        })?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(text),
    ]))
}

// ── Consolidated write tool (2→1) ─────────────────────────────────────────────

/// The invocation action to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum InvocationManageAction {
    /// Open an agent-invocation envelope.
    Open,
    /// Close an open envelope with a terminal disposition.
    Close,
}

/// Consolidated invocation-manage tool — one write tool with an `action` discriminator.
///
/// Collapses `invocation_open` and `invocation_close` into a single MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct InvocationManageInput {
    /// Which invocation action to perform.
    pub action: InvocationManageAction,
    /// Free-form trigger label (e.g. `manual`, `delegated`, `scheduled`). Required for `open`; ignored for `close`.
    #[serde(default)]
    pub trigger_kind: Option<String>,
    /// The cognitive map this invocation runs against, by ref. Required for `open`; ignored for `close`.
    #[serde(default)]
    pub originating_cogmap: Option<String>,
    /// Optional delegating-parent cogmap ref. Used with `open`.
    #[serde(default)]
    pub parent_cogmap: Option<String>,
    /// The invocation to close, by ref (the UUID returned by `open`). Required for `close`; ignored for `open`.
    #[serde(default)]
    pub invocation: Option<String>,
    /// Terminal disposition: `completed`, `failed`, `abandoned`. Required for `close`.
    #[serde(default)]
    pub disposition: Option<Disposition>,
    /// Opaque, agent-defined terminal outcome payload. Used with `close`.
    #[serde(default)]
    pub outcome: Option<serde_json::Value>,
}

/// Dispatch the consolidated invocation-manage tool.
pub async fn invocation_manage(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: InvocationManageInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.action {
        InvocationManageAction::Open => {
            let trigger_kind = input.trigger_kind.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("open requires `trigger_kind`".to_string(), None)
            })?;
            let originating_cogmap = input.originating_cogmap.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "open requires `originating_cogmap`".to_string(),
                    None,
                )
            })?;
            invocation_open(
                svc,
                parts,
                InvocationOpenInput {
                    trigger_kind,
                    originating_cogmap,
                    parent_cogmap: input.parent_cogmap,
                },
            )
            .await
        }
        InvocationManageAction::Close => {
            let invocation = input.invocation.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("close requires `invocation`".to_string(), None)
            })?;
            let disposition = input.disposition.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("close requires `disposition`".to_string(), None)
            })?;
            invocation_close(
                svc,
                parts,
                InvocationCloseInput {
                    invocation,
                    disposition,
                    outcome: input.outcome,
                },
            )
            .await
        }
    }
}

// ── Consolidated read tool (2→1) ───────────────────────────────────────────────

/// The invocation-read view to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum InvocationReadView {
    /// Show one envelope plus its acts by UUID.
    Show,
    /// List envelopes, optionally narrowed.
    List,
}

/// Consolidated invocation-read tool — one read tool with a `view` discriminator.
///
/// Collapses `invocation_show` and `invocation_list` into a single MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct InvocationReadInput {
    /// Which invocation read to perform.
    pub view: InvocationReadView,
    /// The invocation to read, by ref (UUID). Required for `show`; ignored for `list`.
    #[serde(default)]
    pub invocation: Option<String>,
    /// Optional originating cogmap ref to filter by. Used with `list`.
    #[serde(default)]
    pub cogmap: Option<String>,
    /// Optional lifecycle status filter: `open`, `completed`, `failed`, `abandoned`. Used with `list`.
    #[serde(default)]
    pub status: Option<String>,
}

/// Dispatch the consolidated invocation-read tool.
pub async fn invocation_read(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: InvocationReadInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.view {
        InvocationReadView::Show => {
            let invocation = input.invocation.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("show requires `invocation`".to_string(), None)
            })?;
            invocation_show(svc, parts, InvocationShowInput { invocation }).await
        }
        InvocationReadView::List => {
            invocation_list(
                svc,
                parts,
                InvocationListInput {
                    cogmap: input.cogmap,
                    status: input.status,
                },
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::map_err;
    use temper_client::error::ClientError;
    use temper_core::types::invocation::{
        Disposition, InvocationCloseInput, InvocationListInput, InvocationOpenInput,
        InvocationShowInput,
    };

    #[test]
    fn invocation_open_input_deserializes() {
        let json = serde_json::json!({
            "trigger_kind": "agent_run",
            "originating_cogmap": "map-00000000-0000-0000-0005-000000000001",
            "parent_cogmap": "00000000-0000-0000-0005-000000000002"
        });
        let input: InvocationOpenInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.trigger_kind, "agent_run");
        assert_eq!(
            input.parent_cogmap.as_deref(),
            Some("00000000-0000-0000-0005-000000000002")
        );
    }

    #[test]
    fn invocation_close_input_deserializes() {
        let json = serde_json::json!({
            "invocation": "00000000-0000-0000-0005-000000000009",
            "disposition": "failed"
        });
        let input: InvocationCloseInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.disposition, Disposition::Failed);
        assert!(input.outcome.is_none());
    }

    #[test]
    fn invocation_show_input_deserializes() {
        let json = serde_json::json!({ "invocation": "00000000-0000-0000-0005-000000000009" });
        let input: InvocationShowInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.invocation, "00000000-0000-0000-0005-000000000009");
    }

    #[test]
    fn invocation_list_input_deserializes() {
        let json = serde_json::json!({ "status": "open" });
        let input: InvocationListInput = serde_json::from_value(json).unwrap();
        assert!(input.cogmap.is_none());
        assert_eq!(input.status.as_deref(), Some("open"));
    }

    /// The refusal an agent actually reads names **the cognitive map**, which is what
    /// `invocation_open` checks — not the invocation, which does not exist yet when the gate runs.
    /// At the door the bare 403 arrives as `ClientError::Forbidden`; the mapper still speaks the
    /// terse sentence on the caller's behalf. Asserting the absence of the retired wording as well
    /// as the presence of the new one is deliberate — a message that appended the map to the old
    /// sentence would satisfy a contains-check on its own and still be misleading.
    #[test]
    fn the_terse_refusal_names_the_map_not_the_invocation() {
        let rendered = map_err(ClientError::Forbidden, "invocation_open").message;
        assert!(
            rendered.contains("cognitive map"),
            "the refusal must name what was actually checked: {rendered:?}"
        );
        assert!(
            !rendered.contains("this invocation"),
            "the retired wording named a record that does not exist at gate time: {rendered:?}"
        );
    }

    /// The disclosing dialect is passed through, not replaced. At the door the detailed
    /// sentence arrives as `ClientError::ForbiddenDetail` (the wire's `FORBIDDEN_DETAIL`
    /// code); the surface's only job is to not discard it.
    #[test]
    fn a_disclosing_refusal_is_carried_verbatim() {
        let detail = "cannot author cognitive map 0198-…: authorship requires an explicit write \
                      grant on the map, which you do not hold.";
        let rendered = map_err(
            ClientError::ForbiddenDetail {
                message: detail.to_string(),
            },
            "invocation_open",
        )
        .message;
        assert!(
            rendered.contains(detail),
            "the gate's own sentence must survive intact: {rendered:?}"
        );
    }
}
