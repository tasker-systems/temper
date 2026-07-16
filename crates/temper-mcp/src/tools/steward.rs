//! Team-self-cognition steward tools — read the ingest delta, advance the watermark (T4a).
//!
//! `steward_ingest_delta` is a service-direct read (its access gate is
//! `anchor_readable_by_profile` inside the service); `steward_advance_watermark` is a write that
//! dispatches through `DbBackend` — the same path the HTTP handler uses. The cogmap is a decorated
//! ref (a UUID or the `slug-<uuid>` form) resolved via `parse_ref`.

use rmcp::model::CallToolResult;

use temper_core::error::TemperError;
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::steward::{
    AdvanceWatermarkAck, StewardAdvanceWatermarkInput, StewardDeltaInput,
};
use temper_services::backend::DbBackend;
use temper_services::services::steward_service;
use temper_workflow::operations::{AdvanceStewardWatermark, Backend, Surface};

use crate::service::TemperMcpService;

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn map_err(e: TemperError, action: &str) -> rmcp::ErrorData {
    match e {
        // Preserve the NotFound payload — the message already names *which* thing was not found
        // ("cognitive map {id} not found" vs "event {id} is not in cognitive map {id}'s ingest
        // window"). Collapsing both to a fixed "cognitive map not found" masked a real event failure
        // as a cogmap failure (advance_steward_watermark has two distinct NotFound exits: the cogmap
        // gate and the ingest-window check on the target event).
        TemperError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("{action}: {msg}"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        TemperError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: cannot author this cognitive map"),
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

// ── Tool handlers ──────────────────────────────────────────────────────────────

pub async fn steward_ingest_delta(
    svc: &TemperMcpService,
    input: StewardDeltaInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let cogmap = parse_cogmap(&input.cogmap)?;

    let delta = steward_service::ingest_delta(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap,
        input.threshold,
    )
    .await
    .map_err(|e| map_err(TemperError::from(e), "steward_ingest_delta"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&delta),
    )]))
}

pub async fn steward_advance_watermark(
    svc: &TemperMcpService,
    input: StewardAdvanceWatermarkInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let cogmap = parse_cogmap(&input.cogmap)?;

    let cmd = AdvanceStewardWatermark {
        cogmap,
        event_id: input.event_id,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(svc.api_state.pool.clone(), ProfileId::from(profile.id));
    let out = backend
        .advance_steward_watermark(cmd)
        .await
        .map_err(|e| map_err(e, "steward_advance_watermark"))?;

    let ack = AdvanceWatermarkAck {
        cogmap_id: cogmap.uuid(),
        watermark: out.value,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}
