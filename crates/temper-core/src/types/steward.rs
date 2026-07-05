//! Cross-surface types for the team-self-cognition steward's ingest trigger (T4a).
//!
//! The steward runs on a cron cadence but only *acts* when enough new material has landed in its
//! team's contexts since it last ran. [`IngestDelta`] is the answer to "how much has landed since
//! the watermark, and does it clear the threshold" — computed service-direct from
//! `steward_ingest_delta(cogmap, watermark)` and returned over MCP, the API, and the CLI. The
//! watermark itself advances via [`AdvanceWatermarkRequest`] when a run completes.
//!
//! Shared between `temper-api` (OpenAPI schema source), `temper-mcp` (tool params), and
//! `temper-client` (typed request builder). Ids ride the wire as `Uuid` (the id newtypes are an
//! internal detail); both sides re-use these structs rather than string-mirroring a JSON shape.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::workflow_job::ClaimedJob;

/// Default ingest threshold when a caller omits one: the number of newly-created resources in the
/// team's contexts that must accumulate before the steward should run. A calibratable starting
/// point (mirrors the "tuning constants live in one place" discipline); the Eve cron may override
/// it per team.
pub const DEFAULT_STEWARD_INGEST_THRESHOLD: i64 = 5;

/// The ingest delta for a team-self-cognition cogmap since its watermark — the trigger signal the
/// steward's cron pulls. `new_resources` is the gated metric (an *ingest* threshold); `new_events`
/// is the broader activity count for context.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestDelta {
    /// The cogmap whose team-context ingest this measures.
    pub cogmap_id: Uuid,
    /// The watermark the delta was computed against (`kb_cogmaps.steward_watermark_event_id`);
    /// `None` when the steward has never run for this cogmap (delta counts from the beginning).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<Uuid>,
    /// Newly-created resources in the team's contexts since the watermark (the gated ingest signal).
    pub new_resources: i64,
    /// All `kb_events` anchored to the team's contexts since the watermark (total activity).
    pub new_events: i64,
    /// The threshold `new_resources` was compared against (the caller's, or the default).
    pub threshold: i64,
    /// Whether `new_resources >= threshold` — i.e. the steward should run.
    pub exceeds_threshold: bool,
}

/// MCP tool params for `steward_ingest_delta` — read the ingest delta for a cogmap.
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StewardDeltaInput {
    /// The team-self-cognition cogmap ref (decorated or bare UUID).
    pub cogmap: String,
    /// Ingest threshold to gate on; defaults to [`DEFAULT_STEWARD_INGEST_THRESHOLD`] when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<i64>,
}

/// MCP tool params for `steward_advance_watermark` — advance a cogmap's ingest cursor.
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StewardAdvanceWatermarkInput {
    /// The team-self-cognition cogmap ref (decorated or bare UUID).
    pub cogmap: String,
    /// The `kb_events.id` to advance the watermark to — the last event a completed run observed.
    pub event_id: Uuid,
}

/// Request body for `POST /api/steward/{cogmap}/watermark`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct AdvanceWatermarkRequest {
    /// The `kb_events.id` to advance the watermark to.
    pub event_id: Uuid,
}

/// Acknowledgement for a watermark advance.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct AdvanceWatermarkAck {
    /// The cogmap whose watermark advanced.
    pub cogmap_id: Uuid,
    /// The watermark it now holds.
    pub watermark: Uuid,
}

/// One drifted cogmap in a sweep result — the map plus its ingest delta since its own watermark.
/// Ordered most-drifted-first by the sweep (`steward_drift_sweep`).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftSweepRow {
    /// The team-joined cogmap that drifted.
    pub cogmap_id: Uuid,
    /// The watermark the delta was computed against; `None` when the steward has never run for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<Uuid>,
    /// Newly-created resources in the team's contexts since the watermark (the gated ingest signal).
    pub new_resources: i64,
    /// All events anchored to the team's contexts since the watermark (total activity).
    pub new_events: i64,
}

/// Request body for `POST /api/steward/dispatch`. Both optional — server defaults apply
/// (`DEFAULT_STEWARD_INGEST_THRESHOLD`, `DEFAULT_STEWARD_DISPATCH_CAP`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct DispatchTickRequest {
    /// Ingest threshold gating which maps count as drifted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<i64>,
    /// Max maps to claim (and thus sessions to fan out) this tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<i64>,
}

/// Response for a dispatch tick — the jobs claimed for fan-out (one isolated session per entry,
/// each tending a single cogmap).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchTickResponse {
    pub claimed: Vec<ClaimedJob>,
}
