//! Agent-invocation envelope tools — open, close, show, and list.
//!
//! The envelope is an append-only agent-run accountability record. `invocation_open`
//! and `invocation_close` are writes that dispatch through `DbBackend` (the shared
//! write path the HTTP handlers use); `invocation_show` and `invocation_list` are
//! service-direct reads whose access gate lives in the readback SQL (a principal who
//! cannot read the originating cogmap gets an empty result, never an error).

use rmcp::model::CallToolResult;

use temper_core::error::TemperError;
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::invocation::{
    InvocationCloseInput, InvocationListInput, InvocationOpenInput, InvocationShowInput,
};
use temper_core::types::invocation_requests::{InvocationAck, InvocationCloseAck};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CloseInvocation, OpenInvocation, Surface};

use crate::service::TemperMcpService;

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn map_err(e: TemperError, action: &str) -> rmcp::ErrorData {
    match e {
        TemperError::NotFound(_) => {
            rmcp::ErrorData::invalid_params(format!("{action}: invocation not found"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        TemperError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: cannot access this invocation"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

fn parse_cogmap(s: &str) -> Result<CogmapId, rmcp::ErrorData> {
    let uuid = temper_workflow::operations::parse_ref(s)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;
    Ok(CogmapId::from(uuid))
}

fn parse_invocation(s: &str) -> Result<uuid::Uuid, rmcp::ErrorData> {
    Ok(temper_workflow::operations::parse_ref(s)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad invocation ref: {e}"), None))?
        .0)
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

pub async fn invocation_open(
    svc: &TemperMcpService,
    input: InvocationOpenInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let profile_id = ProfileId::from(profile.id);

    let originating_cogmap = parse_cogmap(&input.originating_cogmap)?;
    let parent_cogmap = match input.parent_cogmap.as_deref() {
        Some(p) => Some(parse_cogmap(p)?),
        None => None,
    };

    let cmd = OpenInvocation {
        trigger_kind: input.trigger_kind,
        originating_cogmap,
        parent_cogmap,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(svc.api_state.pool.clone(), profile_id);
    let out = backend
        .open_invocation(cmd)
        .await
        .map_err(|e| map_err(e, "invocation_open"))?;

    let ack = InvocationAck {
        id: out.value,
        invocation_id: out.value,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}

pub async fn invocation_close(
    svc: &TemperMcpService,
    input: InvocationCloseInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let profile_id = ProfileId::from(profile.id);

    let invocation = parse_invocation(&input.invocation)?;

    let disposition = input.disposition;
    let cmd = CloseInvocation {
        invocation,
        disposition,
        outcome: input.outcome.unwrap_or(serde_json::Value::Null),
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(svc.api_state.pool.clone(), profile_id);
    backend
        .close_invocation(cmd)
        .await
        .map_err(|e| map_err(e, "invocation_close"))?;

    let ack = InvocationCloseAck {
        invocation_id: invocation,
        disposition,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}

pub async fn invocation_show(
    svc: &TemperMcpService,
    input: InvocationShowInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let invocation = parse_invocation(&input.invocation)?;

    let view = temper_services::backend::substrate_read::invocation_show_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        invocation,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("invocation_show failed: {e}"), None))?;

    let text = serde_json::to_string_pretty(&view).unwrap_or_else(|_| "null".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

pub async fn invocation_list(
    svc: &TemperMcpService,
    input: InvocationListInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let cogmap = match input.cogmap.as_deref() {
        Some(c) => Some(parse_invocation(c)?),
        None => None,
    };

    let rows = temper_services::backend::substrate_read::invocation_list_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap,
        input.status,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("invocation_list failed: {e}"), None))?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
}
