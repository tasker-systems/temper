//! Cross-surface invocation types. `Disposition` mirrors
//! `temper_substrate::payloads::Disposition`; `NextBackend` maps between them
//! (the `map_edge_kind` pattern) since `temper-core` does not depend on
//! `temper-substrate`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Terminal outcome of an invocation. Mirrors the Postgres / temper-substrate
/// `Disposition`. `open` is NOT representable here — closing requires a
/// terminal value.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invocation.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
// Inline into MCP input schemas — Anthropic tool-use does not resolve `$ref`.
#[cfg_attr(feature = "mcp", schemars(inline))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Completed,
    Failed,
    Abandoned,
}

/// One act of an invocation: a `kb_events` row stamped with this invocation's
/// `invocation_id`. The acts are the per-step accountability trail folded under
/// the envelope's show projection.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invocation.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct InvocationActRow {
    /// `kb_events.id` — the act's stable identity.
    pub event_id: Uuid,
    /// The event type name (e.g. `facet_set`, `relationship_assert`).
    pub event_kind: String,
    /// The entity that emitted this act.
    pub emitter_entity_id: Uuid,
    /// When the act occurred.
    pub occurred_at: DateTime<Utc>,
}

/// The full show projection of an invocation envelope: the `kb_invocations`
/// row plus its acts. Internal ledger pointers (`opened_by_event_id`,
/// `closed_by_event_id`) are NOT exposed — they are dereferenced into `acts`
/// and the open/closed timestamps.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invocation.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct InvocationView {
    /// `kb_invocations.id` — the envelope's stable identity.
    pub id: Uuid,
    /// Lifecycle status: one of `open|completed|failed|abandoned`.
    pub status: String,
    /// What triggered this invocation.
    pub trigger_kind: String,
    /// The cognitive map this invocation runs against.
    pub originating_cogmap_id: Uuid,
    /// The parent cognitive map, when this invocation was spawned beneath another.
    pub parent_cogmap_id: Option<Uuid>,
    /// The entity the invocation is scoped to.
    pub scoped_entity_id: Uuid,
    /// The telos resource governing this invocation.
    pub telos_resource_id: Uuid,
    /// Opaque, agent-defined terminal outcome; `None` while still open.
    pub outcome: Option<serde_json::Value>,
    /// When the invocation opened.
    pub opened_at: DateTime<Utc>,
    /// When the invocation closed; `None` while still open.
    pub closed_at: Option<DateTime<Utc>>,
    /// The acts (stamped events) that occurred under this envelope.
    pub acts: Vec<InvocationActRow>,
}

/// The lighter list-row projection of an invocation envelope (no acts).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invocation.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct InvocationSummary {
    /// `kb_invocations.id` — the envelope's stable identity.
    pub id: Uuid,
    /// Lifecycle status: one of `open|completed|failed|abandoned`.
    pub status: String,
    /// What triggered this invocation.
    pub trigger_kind: String,
    /// The cognitive map this invocation runs against.
    pub originating_cogmap_id: Uuid,
    /// When the invocation opened.
    pub opened_at: DateTime<Utc>,
    /// When the invocation closed; `None` while still open.
    pub closed_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(Disposition::Completed).unwrap(),
            serde_json::json!("completed")
        );
        let back: Disposition = serde_json::from_value(serde_json::json!("abandoned")).unwrap();
        assert_eq!(back, Disposition::Abandoned);
    }

    #[test]
    fn invocation_view_serde_roundtrip() {
        let view = InvocationView {
            id: Uuid::from_u128(1),
            status: "open".to_string(),
            trigger_kind: "agent_run".to_string(),
            originating_cogmap_id: Uuid::from_u128(2),
            parent_cogmap_id: None,
            scoped_entity_id: Uuid::from_u128(3),
            telos_resource_id: Uuid::from_u128(4),
            outcome: None,
            opened_at: Utc::now(),
            closed_at: None,
            acts: vec![InvocationActRow {
                event_id: Uuid::from_u128(5),
                event_kind: "facet_set".to_string(),
                emitter_entity_id: Uuid::from_u128(6),
                occurred_at: Utc::now(),
            }],
        };
        let json = serde_json::to_string(&view).expect("serialize");
        // open invocation: null closed_at survives the round-trip
        assert!(json.contains("\"closed_at\":null"), "json: {json}");
        let back: InvocationView = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, view);
    }
}
