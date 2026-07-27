//! Cross-surface types for the team-self-cognition steward's ingest trigger (T4a).
//!
//! The steward runs on a cron cadence but only *acts* when enough new material has landed in its
//! team's contexts **or its scope itself moved**. [`IngestDelta`] is the answer to "how much has
//! landed since the watermark, has the boundary shifted, and does either clear the bar" — computed
//! service-direct from `steward_ingest_delta(cogmap, watermark, fingerprint)` and returned over MCP,
//! the API, and the CLI. Both cursors advance via [`AdvanceWatermarkRequest`] when a run completes.
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
/// steward's cron pulls. `new_resources` is the counted ingest metric; `new_events` is the broader
/// activity count for context; `boundary_moved` is the *uncounted* signal — the steward's scope
/// itself changed shape.
///
/// The two signals are independent by construction. The counts measure events **inside**
/// `steward_team_contexts` (team-OWNED ∪ SHARED contexts); the boundary is that set itself. Sharing a
/// context into a joined team writes a `kb_team_contexts` row and emits **no event**
/// (`context_service`: "Context creation is a plain INSERT (no event emission — product decision 5:
/// contexts are infrastructure)"), so a whole context of material can enter scope with every count
/// still zero. That is what [`Self::boundary_fingerprint`] and [`Self::boundary_moved`] observe.
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
    /// The latest `kb_events.id` in the delta window — the value a completed tick passes straight to
    /// `steward_advance_watermark` to mark this delta ingested. `None` when the delta is empty (no
    /// events since the watermark), in which case there is nothing to advance to and the tick skips
    /// the advance. Because `kb_events.id` is uuidv7 (its byte order is time order), the greatest
    /// id in the window is the newest event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_event_id: Option<Uuid>,
    /// The threshold `new_resources` was compared against (the caller's, or the default).
    pub threshold: i64,
    /// **The decision: should the steward run?** `new_resources >= threshold || boundary_moved`.
    ///
    /// It is deliberately NOT a restatement of the count comparison, so a `true` sitting next to
    /// `new_resources: 0` is not a bug — it is the boundary arm firing. Read the two inputs
    /// separately when you want the reason: `new_resources`/`threshold` for the counted arm,
    /// [`Self::boundary_moved`] for the uncounted one.
    pub exceeds_threshold: bool,
    /// The fingerprint of the cogmap's change-detection boundary **as computed for this read** — the
    /// value a completed run passes back to `steward_advance_watermark` (alongside `max_event_id`) to
    /// record the boundary it actually processed. Carry this one; do not recompute or substitute
    /// another read's, or a boundary move that happened in between is silently swallowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_fingerprint: Option<String>,
    /// Whether the boundary changed since the fingerprint the cogmap has stored — i.e. contexts
    /// entered or left the steward's scope without emitting a countable event. `true` when nothing
    /// was ever snapshotted (a NULL stored fingerprint is "unknown", and unknown means run).
    pub boundary_moved: bool,
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
    /// The `kb_events.id` to advance the watermark to — the last event a completed run observed
    /// ([`IngestDelta::max_event_id`]).
    ///
    /// **Optional, and its absence is a legitimate advance, not a degraded one.** A run whose delta
    /// fired on the boundary arm alone has an empty event window and therefore no id to advance to;
    /// omitting it leaves the watermark exactly where it was and stores only the boundary cursor.
    /// Requiring it here is what made that run unable to record anything, so it re-fired every tick
    /// forever. A supplied id is still validated against the cogmap's ingest window as strictly as
    /// before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<Uuid>,
    /// The boundary fingerprint the run observed — [`IngestDelta::boundary_fingerprint`] from the
    /// delta it processed. **Supply it.**
    ///
    /// Absence is a fallback, not an equivalent: the server then stores the boundary as it stands at
    /// write time rather than as this run read it. Both settle the boundary — neither leaves it NULL,
    /// because a caller that never sends one would otherwise re-fire on every tick forever — but the
    /// fallback silently absorbs any boundary change that happened *during* the run, marking as seen
    /// a context the run never looked at. Carrying the delta's value is the only way that mid-run
    /// change survives to the next tick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_fingerprint: Option<String>,
}

/// Request body for `POST /api/steward/{cogmap}/watermark`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct AdvanceWatermarkRequest {
    /// The `kb_events.id` to advance the watermark to. Optional: a boundary-only advance (an empty
    /// event window) supplies none and the watermark stays put. See
    /// [`StewardAdvanceWatermarkInput::event_id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<Uuid>,
    /// The boundary fingerprint the run observed ([`IngestDelta::boundary_fingerprint`]). Optional,
    /// but supplying it is the correct path — see
    /// [`StewardAdvanceWatermarkInput::boundary_fingerprint`] for what the fallback gives up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_fingerprint: Option<String>,
}

/// Acknowledgement for a watermark advance — **the cursors as stored**, read back from the UPDATE
/// itself (`RETURNING`), never an echo of the request.
///
/// The difference is load-bearing. Both fields are optional on the way in and both have a server-side
/// rule applied to them (an absent `event_id` leaves the watermark put; an absent
/// `boundary_fingerprint` falls back to the write-time boundary), so echoing the input would report a
/// state that may not exist. A caller — and a test — can compare these against what it sent and see
/// exactly which rule fired.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct AdvanceWatermarkAck {
    /// The cogmap whose cursors were written.
    pub cogmap_id: Uuid,
    /// The watermark now stored. `None` when the cogmap has never had one — which a boundary-only
    /// advance (no `event_id`) does not change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<Uuid>,
    /// The boundary fingerprint now stored. Never `None` after a successful advance: the fallback
    /// computes one when the caller supplies none, precisely so the boundary cannot stay unsnapshotted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_fingerprint: Option<String>,
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
    /// Newly-created resources in the team's contexts since the watermark (the counted ingest signal).
    pub new_resources: i64,
    /// All events anchored to the team's contexts since the watermark (total activity).
    pub new_events: i64,
    /// Whether this map is in the sweep on the *boundary* arm rather than the counted one — its
    /// change-detection scope moved without emitting countable events. Explains a row whose
    /// `new_resources` is below the sweep's threshold (including zero).
    ///
    /// The fingerprint *value* deliberately does not ride this row. The only consumer of a
    /// fingerprint is `steward_advance_watermark`, and the value it must store is the one from the
    /// delta the run actually processed ([`IngestDelta::boundary_fingerprint`]) — a sweep-time value
    /// is older, and storing it would mark a boundary state no run ever saw, swallowing any move
    /// between sweep and run.
    pub boundary_moved: bool,
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
    /// The correlation the server parsed out of `x-steward-correlation-id` and stamped onto every
    /// claimed job — echoed so the caller can assert its tick id survived, rather than assume it.
    /// `None` when the header was absent or not a UUID (both self-root; neither is an error).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<Uuid>,
}
