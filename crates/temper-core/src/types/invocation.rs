//! Cross-surface invocation types. `Disposition` mirrors
//! `temper_substrate::payloads::Disposition`; `NextBackend` maps between them
//! (the `map_edge_kind` pattern) since `temper-core` does not depend on
//! `temper-substrate`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::authorship::AgentAuthorship;

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

impl TryFrom<&str> for Disposition {
    type Error = String;

    /// Parse a terminal `kb_invocations.status` into its disposition.
    ///
    /// There is no `disposition` column: `invocation_close` writes the disposition into
    /// `status` directly (`outcome` holds only the caller's opaque payload, despite what
    /// the comment on `kb_invocations.outcome` in the canonical schema migration claims).
    ///
    /// `open` and any unknown value are errors, not `None`: the column's CHECK
    /// constraint admits exactly `open|completed|failed|abandoned`, so an unparseable
    /// terminal status means an invariant broke and must be loud. Callers map `open`
    /// to `None` before calling.
    fn try_from(status: &str) -> Result<Self, Self::Error> {
        match status {
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(format!(
                "not a terminal disposition: `{other}` (expected completed|failed|abandoned)"
            )),
        }
    }
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
    /// The invocation this act is correlated under (`kb_events.invocation_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<Uuid>,
    /// The graded agent authorship of this act (decoded from `kb_events.metadata`); `None` for an
    /// act with no authorship attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorship: Option<AgentAuthorship>,
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
    /// The terminal disposition, derived from `status`. `None` while the invocation is open.
    ///
    /// There is no `disposition` column. `invocation_close` writes the disposition into
    /// `status`; `outcome` holds only the caller's opaque payload, despite what the comment
    /// on `kb_invocations.outcome` in the canonical schema migration claims. Surfacing it
    /// under its own name here makes "did the close take?" answerable without knowing that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<Disposition>,
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

// ── MCP surface inputs ──────────────────────────────────────────────────────
//
// Cogmap/invocation ids arrive as **string refs** (parse_ref'd in the tool — a bare
// UUID passes parse_ref's trailing-UUID-only resolver). Mirrors `CogmapShapeInput`
// for derives/attributes; doc comments become the MCP tool's field descriptions.

/// MCP input for `invocation_open`. Opens an agent-run accountability envelope; the
/// server mints the id and returns it (feed it into `invocation_close`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct InvocationOpenInput {
    /// Free-form trigger label (e.g. `manual`, `delegated`, `scheduled`).
    pub trigger_kind: String,
    /// The cognitive map this invocation runs against, by ref (UUID or `slug-<uuid>`).
    pub originating_cogmap: String,
    /// Optional delegating-parent cogmap ref; omit when not spawned beneath another.
    pub parent_cogmap: Option<String>,
}

/// MCP input for `invocation_close`. Terminates an open envelope with a disposition
/// and opaque outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct InvocationCloseInput {
    /// The invocation to close, by ref (the UUID returned by `invocation_open`).
    pub invocation: String,
    /// Terminal disposition: one of `completed`, `failed`, `abandoned`.
    pub disposition: Disposition,
    /// Opaque, agent-defined terminal outcome payload; omit for none.
    pub outcome: Option<serde_json::Value>,
}

/// MCP input for `invocation_show`. Reads one envelope plus its acts by raw UUID.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct InvocationShowInput {
    /// The invocation to read, by ref (UUID or `slug-<uuid>`).
    pub invocation: String,
}

/// MCP input for `invocation_list`. Lists envelopes, optionally narrowed by
/// originating cogmap and/or lifecycle status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct InvocationListInput {
    /// Optional originating cogmap ref to filter by; omit for all maps.
    pub cogmap: Option<String>,
    /// Optional lifecycle status filter: one of `open`, `completed`, `failed`, `abandoned`.
    pub status: Option<String>,
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
            disposition: None,
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
                invocation_id: Some(Uuid::from_u128(1)),
                authorship: Some(AgentAuthorship {
                    reasoning: Some("seeded".to_string()),
                    confidence: crate::types::ConfidenceBand::Probable,
                    rationale: None,
                    persona: None,
                    model: None,
                }),
            }],
        };
        let json = serde_json::to_string(&view).expect("serialize");
        // open invocation: null closed_at survives the round-trip
        assert!(json.contains("\"closed_at\":null"), "json: {json}");
        // an open invocation has no disposition; the key is omitted, not emitted as null
        assert!(!json.contains("disposition"), "json: {json}");
        // per-act authorship rides on the wire row
        assert!(json.contains("\"confidence\":\"probable\""), "json: {json}");
        let back: InvocationView = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, view);
    }

    #[test]
    fn invocation_view_serializes_disposition_when_closed() {
        let view = InvocationView {
            id: Uuid::from_u128(1),
            status: "completed".to_string(),
            disposition: Some(Disposition::Completed),
            trigger_kind: "agent_run".to_string(),
            originating_cogmap_id: Uuid::from_u128(2),
            parent_cogmap_id: None,
            scoped_entity_id: Uuid::from_u128(3),
            telos_resource_id: Uuid::from_u128(4),
            outcome: None,
            opened_at: Utc::now(),
            closed_at: Some(Utc::now()),
            acts: vec![],
        };
        let json = serde_json::to_string(&view).expect("serialize");
        assert!(
            json.contains("\"disposition\":\"completed\""),
            "json: {json}"
        );
        let back: InvocationView = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, view);
    }

    #[test]
    fn invocation_act_row_omits_empty_authorship() {
        // An unauthored act serializes without invocation_id/authorship noise (both skip when None).
        let row = InvocationActRow {
            event_id: Uuid::from_u128(7),
            event_kind: "resource_created".to_string(),
            emitter_entity_id: Uuid::from_u128(8),
            occurred_at: Utc::now(),
            invocation_id: None,
            authorship: None,
        };
        let json = serde_json::to_string(&row).expect("serialize");
        assert!(!json.contains("authorship"), "json: {json}");
        assert!(!json.contains("invocation_id"), "json: {json}");
    }

    #[test]
    fn invocation_open_input_deserializes() {
        let json = serde_json::json!({
            "trigger_kind": "agent_run",
            "originating_cogmap": "my-map-00000000-0000-0000-0005-000000000001",
            "parent_cogmap": "00000000-0000-0000-0005-000000000002"
        });
        let input: InvocationOpenInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.trigger_kind, "agent_run");
        assert_eq!(
            input.originating_cogmap,
            "my-map-00000000-0000-0000-0005-000000000001"
        );
        assert_eq!(
            input.parent_cogmap.as_deref(),
            Some("00000000-0000-0000-0005-000000000002")
        );
    }

    #[test]
    fn invocation_open_input_omits_parent() {
        let json = serde_json::json!({
            "trigger_kind": "manual",
            "originating_cogmap": "00000000-0000-0000-0005-000000000001"
        });
        let input: InvocationOpenInput = serde_json::from_value(json).unwrap();
        assert!(input.parent_cogmap.is_none());
    }

    #[test]
    fn invocation_close_input_deserializes() {
        let json = serde_json::json!({
            "invocation": "00000000-0000-0000-0005-000000000009",
            "disposition": "completed",
            "outcome": { "summary": "done" }
        });
        let input: InvocationCloseInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.invocation, "00000000-0000-0000-0005-000000000009");
        assert_eq!(input.disposition, Disposition::Completed);
        assert_eq!(
            input.outcome,
            Some(serde_json::json!({ "summary": "done" }))
        );
    }

    #[test]
    fn invocation_close_input_omits_outcome() {
        let json = serde_json::json!({
            "invocation": "00000000-0000-0000-0005-000000000009",
            "disposition": "abandoned"
        });
        let input: InvocationCloseInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.disposition, Disposition::Abandoned);
        assert!(input.outcome.is_none());
    }

    #[test]
    fn invocation_show_input_deserializes() {
        let json = serde_json::json!({
            "invocation": "00000000-0000-0000-0005-000000000009"
        });
        let input: InvocationShowInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.invocation, "00000000-0000-0000-0005-000000000009");
    }

    #[test]
    fn invocation_list_input_deserializes_filters() {
        let json = serde_json::json!({
            "cogmap": "00000000-0000-0000-0005-000000000001",
            "status": "open"
        });
        let input: InvocationListInput = serde_json::from_value(json).unwrap();
        assert_eq!(
            input.cogmap.as_deref(),
            Some("00000000-0000-0000-0005-000000000001")
        );
        assert_eq!(input.status.as_deref(), Some("open"));
    }

    #[test]
    fn invocation_list_input_all_optional() {
        let json = serde_json::json!({});
        let input: InvocationListInput = serde_json::from_value(json).unwrap();
        assert!(input.cogmap.is_none());
        assert!(input.status.is_none());
    }
}

#[cfg(test)]
mod disposition_tests {
    use super::*;

    #[test]
    fn disposition_parses_every_terminal_status() {
        assert_eq!(
            Disposition::try_from("completed").unwrap(),
            Disposition::Completed
        );
        assert_eq!(
            Disposition::try_from("failed").unwrap(),
            Disposition::Failed
        );
        assert_eq!(
            Disposition::try_from("abandoned").unwrap(),
            Disposition::Abandoned
        );
    }

    #[test]
    fn disposition_rejects_open_and_unknown() {
        // `open` is not a disposition — it is the absence of one.
        assert!(Disposition::try_from("open").is_err());
        // An unknown status means the DB CHECK was violated: escalate, never silently degrade.
        assert!(Disposition::try_from("cancelled").is_err());
    }
}
