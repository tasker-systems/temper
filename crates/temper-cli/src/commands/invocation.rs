//! `temper invocation open|close|show|list` — surface commands for the
//! agent-invocation envelope. Each resolves string refs → substrate UUIDs,
//! builds the wire request, dispatches one API call, and renders the result.

use crate::cli::DispositionArg;
use crate::error::{Result, TemperError};
use crate::format::OutputFormat;
use temper_core::types::invocation_requests::{
    CloseInvocationRequest, InvocationCloseAck, OpenInvocationRequest,
};

/// `temper invocation open --cogmap <ref> [--parent <ref>] --trigger-kind <kind>`.
pub fn open(
    cogmap_ref: &str,
    parent_ref: Option<&str>,
    trigger_kind: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let originating_cogmap = temper_workflow::operations::parse_ref(cogmap_ref)?.0;
    let parent_cogmap = parent_ref
        .map(|p| temper_workflow::operations::parse_ref(p).map(|r| r.0))
        .transpose()?;

    let req = OpenInvocationRequest {
        trigger_kind: trigger_kind.to_string(),
        originating_cogmap,
        parent_cogmap,
    };

    let ack = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::invocation::open_api(client, &req).await })
    })?;

    let rendered = crate::format::render(&ack, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper invocation close <ref> --disposition <d> [--outcome <json>]`.
pub fn close(
    invocation_ref: &str,
    disposition: DispositionArg,
    outcome: Option<&str>,
    fmt: OutputFormat,
) -> Result<()> {
    let invocation_id = temper_workflow::operations::parse_ref(invocation_ref)?.0;

    let outcome = match outcome {
        Some(raw) => serde_json::from_str(raw)
            .map_err(|e| TemperError::Config(format!("--outcome is not valid JSON: {e}")))?,
        None => serde_json::Value::Null,
    };

    let disposition = disposition.to_core();
    let req = CloseInvocationRequest {
        disposition,
        outcome,
    };

    crate::actions::runtime::with_client(|client| {
        Box::pin(
            async move { crate::actions::invocation::close_api(client, invocation_id, &req).await },
        )
    })?;

    // The close endpoint returns 204 No Content; surface a typed acknowledgement (shared with the
    // MCP surface so both emit the same shape).
    let ack = InvocationCloseAck {
        invocation_id,
        disposition,
    };
    let rendered = crate::format::render(&ack, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper invocation show <ref>`.
pub fn show(invocation_ref: &str, fmt: OutputFormat) -> Result<()> {
    let invocation_id = temper_workflow::operations::parse_ref(invocation_ref)?.0;

    let view = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::invocation::show_api(client, invocation_id).await })
    })?;

    let rendered = crate::format::render(&view, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper invocation list [--cogmap <ref>] [--status <s>]`.
pub fn list(cogmap_ref: Option<&str>, status: Option<&str>, fmt: OutputFormat) -> Result<()> {
    let cogmap = cogmap_ref
        .map(|c| temper_workflow::operations::parse_ref(c).map(|r| r.0))
        .transpose()?;
    let status = status.map(str::to_string);

    let rows = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::invocation::list_api(client, cogmap, status).await })
    })?;

    let rendered = crate::format::render(&rows, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}
