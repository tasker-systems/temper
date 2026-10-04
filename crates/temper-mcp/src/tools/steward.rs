//! Team-self-cognition steward tools — read the ingest delta, advance the watermark (T4a).
//!
//! Execution crosses the DEPLOYED API over the wire (beat 5 — the last family off the
//! direct binding): `steward_ingest_delta` forwards to `GET /api/steward/{cogmap}/delta`
//! and `steward_advance_watermark` to `POST /api/steward/{cogmap}/watermark`
//! (`temper-api/src/handlers/steward.rs`), as per-request temper-client relays built from
//! the request's `Parts`. The routes make the identical service/backend calls the direct
//! binding made — `steward_service::ingest_delta` (its `anchor_readable_by_profile` gate
//! inside) and `DbBackend::advance_steward_watermark` (auth-before-write inside) — so the
//! gates are unchanged; they simply run at the API, behind its Level 1 + 2.
//!
//! The cogmap is a decorated ref (a UUID or the `slug-<uuid>` form) parsed MCP-locally to
//! its trailing UUID via `parse_ref` — a pure parse, no read, so not a retained resolver.
//! The advance's `origin` is no longer this tool's to stamp: the route takes it from the
//! request's resolved surface, which the relay's trusted carrier makes `@mcp`.
//!
//! # Declared parity delta (per the register's G3c delta format)
//!
//! - **NotFound prefix drops**: the direct map prefixed `{action}: ` on both tools' NotFound
//!   arms (the delta's unreadable/absent cogmap; the advance's cogmap exit and its
//!   ingest-window exit); the door's `ClientError::NotFound` carries the server's own
//!   sentence, and the door does not re-apply a prefix the direct tool applied. Kind
//!   (`invalid_params`) and gate identical; the advance's two NotFound exits stay
//!   distinguishable by their sentences.
//!
//! NOT a delta: the advance's disclosing 403 (`ForbiddenDetail`) keeps the direct face
//! byte-for-byte — `{action}: ` prefix, the backend's sentence, INVALID_REQUEST — per the
//! reblock family's precedent. The terse `Forbidden` arm is kept for arm-completeness; this
//! backend only refuses with the detailed variant.

use rmcp::model::CallToolResult;

use temper_client::error::ClientError;
use temper_core::types::steward::{StewardAdvanceWatermarkInput, StewardDeltaInput};
use uuid::Uuid;

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

/// Client errors into rmcp errors, in the G3c/G3d/G4 mapping idiom (see the module header
/// for the declared delta). The deployment's refusal kinds ride `AcrossAuth` before this.
fn map_err(e: ClientError, action: &str) -> rmcp::ErrorData {
    match e {
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        // A refusal that named the capability it withheld — carry the gate's own sentence,
        // under the direct binding's prefix and kind.
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

fn parse_cogmap(s: &str) -> Result<Uuid, rmcp::ErrorData> {
    Ok(temper_workflow::operations::parse_ref(s)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0)
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

pub async fn steward_ingest_delta(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: StewardDeltaInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let cogmap = parse_cogmap(&input.cogmap)?;

    let delta = svc
        .relay_client(parts)?
        .steward()
        .delta(cogmap, input.threshold)
        .await
        .across_auth(|e| map_err(e, "steward_ingest_delta"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&delta)),
    ]))
}

pub async fn steward_advance_watermark(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: StewardAdvanceWatermarkInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let cogmap = parse_cogmap(&input.cogmap)?;

    // Render the cursors AS STORED. The agent needs to see which of its two optional inputs the
    // server filled in for it — the route's ack is built from what the UPDATE stored, and
    // re-assembling it from `input` here would hide exactly that.
    let ack = svc
        .relay_client(parts)?
        .steward()
        .advance_watermark(cogmap, input.event_id, input.boundary_fingerprint)
        .await
        .across_auth(|e| map_err(e, "steward_advance_watermark"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&ack)),
    ]))
}
