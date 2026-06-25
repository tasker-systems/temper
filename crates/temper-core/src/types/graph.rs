//! Neutral structural edge taxonomy — the `edge_kind` / `edge_polarity`
//! primitives shared by the relationship wire types (`relationship_requests`,
//! `relationship_events`).
//!
//! The `DocType`-dependent half of the graph (edge declarations, traversal
//! rows, subgraph responses) lives in `temper_workflow::types::graph`.

use serde::{Deserialize, Serialize};

// ─── Structural Edge Typing (SSTorytime four-type taxonomy) ─────────────────

/// Structural edge type — the four Semantic-Spacetime primitives. Each kind
/// carries a distinct traversal algebra:
/// - `Contains`  — transitive (composition / part-of participation)
/// - `LeadsTo`   — antisymmetric, causal/temporal order
/// - `Near`      — symmetric (proximity / similarity)
/// - `Express`   — leaf attribute (has-property)
///
/// Mirrors the Postgres `edge_kind` enum.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
// Inline the variants into every MCP input schema rather than emitting a
// `$ref` into `$defs`. The Anthropic tool-use layer does not resolve
// `$ref`/`$defs`, so a referenced scalar enum reaches the model with no type
// signal and is sent back as `null`. Inlining surfaces the `enum` values
// directly on the field. See tasks/review-mcp-assert-relationship-edge-issues.
#[cfg_attr(feature = "mcp", schemars(inline))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "edge_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Express,
    Contains,
    LeadsTo,
    Near,
}

/// Edge direction sign. `source → target` as asserted may run *with* the
/// structural arrow (`Forward`) or *against* it (`Inverse`) — e.g. a
/// `depends_on` edge is asserted source=dependant/target=dependency, but the
/// causal arrow runs dependency→dependant, so it is `Inverse` `LeadsTo`.
/// `Near` is symmetric: always `Forward`.
///
/// Mirrors the Postgres `edge_polarity` enum.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
// Inline into MCP input schemas — see the note on `EdgeKind` above.
#[cfg_attr(feature = "mcp", schemars(inline))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "edge_polarity", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Forward,
    Inverse,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── EdgeKind / Polarity ─────────────────────────────────────────────

    #[test]
    fn edge_kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&EdgeKind::LeadsTo).unwrap(),
            "\"leads_to\""
        );
        assert_eq!(serde_json::to_string(&EdgeKind::Near).unwrap(), "\"near\"");
    }

    #[test]
    fn polarity_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&Polarity::Inverse).unwrap(),
            "\"inverse\""
        );
    }
}
