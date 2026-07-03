//! R5 element event-trail: a time-ordered history of events for a single graph
//! element (a node/resource, or an edge). Carries the canonical event-type
//! **string** (`kb_event_types.name`) on the wire, not the substrate `EventKind`
//! (Copy-only/no-serde, and lives in the wrong crate for this boundary).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which element a trail belongs to.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "element_trail.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Node,
    Edge,
}

/// A single event on an element's timeline.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "element_trail.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ElementEvent {
    pub event_id: Uuid,
    /// Canonical event-type name (kb_event_types.name), e.g. "relationship.asserted".
    pub kind: String,
    /// The authoring agent entity (kb_events.emitter_entity_id).
    pub actor_entity_id: Uuid,
    /// ISO-8601 emission time (kb_events.occurred_at).
    pub occurred_at: String,
    /// ConfidenceBand from event metadata, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
}

/// A time-ordered event trail for one node or edge.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "element_trail.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct EventTrail {
    pub element_kind: ElementKind,
    pub element_id: Uuid,
    pub events: Vec<ElementEvent>,
}
