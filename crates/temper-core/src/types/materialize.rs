//! Cross-surface types for cron-driven region materialization on a drift threshold (T4b).
//!
//! Region materialization is the substrate's pure function, NOT the steward's authored act (the
//! determinism reframe): a separate cadence from the ingest→steward trigger. [`MaterializeDelta`] is
//! the answer to "how many formation events have landed on this cogmap since it was last
//! materialized, and does that clear the threshold" — computed service-direct from
//! `formation_touched_count_since(cogmap, shape_materialized_event_id)`. The trigger
//! ([`MaterializeRequest`]) re-checks that gate and, only when it clears, invokes the existing
//! incremental-materialize path; below threshold it is a safe no-op ([`MaterializeAck::materialized`]
//! `= false`).
//!
//! Shared between `temper-api` (OpenAPI schema source), `temper-mcp` (tool params), and
//! `temper-client` (typed request builder). Ids ride the wire as `Uuid`; both sides re-use these
//! structs rather than string-mirroring a JSON shape.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Default materialize threshold when a caller omits one: the number of formation-affecting events
/// (created resources, asserted/retyped/reweighted/folded edges, facets, block edits) that must
/// accumulate on the cogmap since its last materialize before re-materialization is worth its cost. A
/// calibratable starting point (mirrors the "tuning constants live in one place" discipline); the
/// materialize cron may override it per map.
pub const DEFAULT_MATERIALIZE_THRESHOLD: i64 = 5;

/// The lens the materialize cron re-materializes: the canonical default region-producing lens seeded
/// for every cogmap (`kb_cogmap_lenses.name`, `20260624000003_canonical_seed.sql`). The MVP tends one
/// lens per map; naming it here keeps the "materialize which perspective" choice in one place.
pub const DEFAULT_MATERIALIZE_LENS: &str = "telos-default";

/// The materialize delta for a cognitive map since its last materialize — the trigger signal the
/// region-materialize cron pulls. `formation_events` is the gated metric (a structural-drift count);
/// `exceeds_threshold` is the "should this re-materialize" answer.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "materialize.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializeDelta {
    /// The cogmap this delta measures.
    pub cogmap_id: Uuid,
    /// The materialize watermark the delta was computed against (`kb_cogmaps.shape_materialized_event_id`);
    /// `None` when the cogmap has never been materialized (delta counts from the beginning).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<Uuid>,
    /// Formation-affecting events anchored to the cogmap since the watermark (the gated drift signal).
    pub formation_events: i64,
    /// The threshold `formation_events` was compared against (the caller's, or the default).
    pub threshold: i64,
    /// Whether `formation_events >= threshold` — i.e. the cogmap should re-materialize.
    pub exceeds_threshold: bool,
}

/// MCP tool params for `cogmap_materialize_delta` — read the materialize delta for a cogmap.
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeDeltaInput {
    /// The cognitive map ref (decorated or bare UUID).
    pub cogmap: String,
    /// Materialize threshold to gate on; defaults to [`DEFAULT_MATERIALIZE_THRESHOLD`] when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<i64>,
}

/// MCP tool params for `cogmap_materialize` — re-materialize a cogmap when its delta clears the threshold.
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeTriggerInput {
    /// The cognitive map ref (decorated or bare UUID).
    pub cogmap: String,
    /// Materialize threshold to gate on; defaults to [`DEFAULT_MATERIALIZE_THRESHOLD`] when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<i64>,
}

/// Request body for `POST /api/cognitive-maps/{cogmap}/materialize`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct MaterializeRequest {
    /// Materialize threshold to gate on; defaults to [`DEFAULT_MATERIALIZE_THRESHOLD`] when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<i64>,
}

/// Outcome of a materialize trigger. When `materialized` is false the delta was below threshold and
/// nothing ran (the idempotent no-op); when true, `regions` + `membership_fingerprint` describe the
/// materialize that ran.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "materialize.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeAck {
    /// The cogmap the trigger targeted.
    pub cogmap_id: Uuid,
    /// Whether a materialize actually ran (`formation_events >= threshold`). False = idempotent no-op.
    pub materialized: bool,
    /// The delta observed when the gate was evaluated.
    pub formation_events: i64,
    /// The threshold the delta was compared against.
    pub threshold: i64,
    /// Regions produced by the materialize — `Some` only when `materialized` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regions: Option<i64>,
    /// The full-clustering membership fingerprint the materialize produced — `Some` only when `materialized`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership_fingerprint: Option<String>,
}
