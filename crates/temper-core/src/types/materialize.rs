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

/// The default region-producing lens for a **context** (`20260712000050_workflow_default_lens.sql`).
/// The regime switch is the lens's `w_cos`: `telos-default` holds it at 0 (the declared graph is the
/// whole signal), `workflow-default` at 1 (the embedding is the primary evidence of regionality) —
/// which is what lets a context, carrying no facets and almost no declared edges, form regions at all.
pub const DEFAULT_CONTEXT_LENS: &str = "workflow-default";

/// The default lens for an anchor, by kind. The two regimes are one producer with different weights,
/// so "which lens" is the ONLY thing that differs between materializing a context and a cogmap — and
/// picking it in one place is what keeps a caller from silently region-producing a context under the
/// declared-graph-only lens (which would form nothing, since contexts carry no facets).
pub fn default_lens_for(anchor: crate::types::home::HomeAnchor) -> &'static str {
    match anchor {
        crate::types::home::HomeAnchor::Context(_) => DEFAULT_CONTEXT_LENS,
        crate::types::home::HomeAnchor::Cogmap(_) => DEFAULT_MATERIALIZE_LENS,
    }
}

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

/// MCP tool params for `context_materialize` — re-materialize a CONTEXT's regions when its delta
/// clears the threshold (T8). The context-addressed peer of [`MaterializeTriggerInput`].
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMaterializeInput {
    /// The context, by context ref (`@me/<slug>`, `+<team>/<slug>`, or a UUID).
    pub context: String,
    /// Materialize threshold to gate on; defaults to [`DEFAULT_MATERIALIZE_THRESHOLD`] when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<i64>,
}

/// Request body for `POST /api/cognitive-maps/{cogmap}/materialize` and
/// `POST /api/contexts/{context}/materialize` — the anchor rides the path, so the body is the same.
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
///
/// ## Why `cogmap_id` survives beside the anchor pair
///
/// T8 made this command anchor-addressed (a context materializes too), so the target is now
/// `anchor_table` + `anchor_id`. `cogmap_id` is kept — and still populated whenever the anchor IS a
/// cogmap — deliberately, because this is a **wire type on a deployed instance**: the temper-rb gem
/// `raise`s on an unknown attribute *and* on a missing required one, and the generated TS is
/// consumed by a UI that ships on its own cadence. Dropping `cogmap_id` would hard-fail an older
/// client on the cogmap path it already uses.
///
/// A client old enough to depend on `cogmap_id` cannot address a context (the route did not exist),
/// so it never receives an ack where the field is absent. New clients read the anchor pair and
/// ignore `cogmap_id`; it goes away with the rest of the `cogmap_*` naming at M3.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "materialize.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeAck {
    /// The anchor table the trigger targeted — `kb_contexts` or `kb_cogmaps`.
    pub anchor_table: String,
    /// The anchor the trigger targeted.
    pub anchor_id: Uuid,
    /// Legacy alias for `anchor_id`, present iff the anchor is a cogmap. Prefer the anchor pair;
    /// see the type's docs for why this is still here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cogmap_id: Option<Uuid>,
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

impl MaterializeAck {
    /// Build an ack for `anchor`, filling the anchor pair and the legacy `cogmap_id` from one source
    /// so the two can never disagree. The below-threshold no-op is the `materialized = false` case:
    /// `regions` / `membership_fingerprint` stay `None`.
    pub fn new(
        anchor: crate::types::home::HomeAnchor,
        materialized: bool,
        formation_events: i64,
        threshold: i64,
    ) -> Self {
        Self {
            anchor_table: anchor.table().to_owned(),
            anchor_id: anchor.uuid(),
            cogmap_id: anchor.cogmap_id().map(|m| m.uuid()),
            materialized,
            formation_events,
            threshold,
            regions: None,
            membership_fingerprint: None,
        }
    }

    /// Attach the outcome of a materialize that actually ran.
    #[must_use]
    pub fn with_outcome(mut self, regions: i64, membership_fingerprint: String) -> Self {
        self.regions = Some(regions);
        self.membership_fingerprint = Some(membership_fingerprint);
        self
    }
}
