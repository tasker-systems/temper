use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::graph::{EdgeKind, Polarity};

/// Which home a node is bound to — drives the Atlas fill-vs-outline encoding
/// (cogmap-homed = filled chip, context-homed = outlined chip). A resource has
/// exactly one home (`kb_resource_homes.resource_id` is unique); this
/// distinguishes the two anchor kinds.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum NodeHome {
    Context,
    Cogmap,
}

/// A node on the Atlas canvas. `doc_type` is the raw, optional `kb_properties`
/// value (a node may carry none); the UI maps it to a hue with a fallback.
/// `degree` is the node's total visible edge count (sizing hint). `salience`
/// is region-derived and may be `None` in the neighborhood tier.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasNode {
    pub id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    pub home: NodeHome,
    pub degree: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salience: Option<f64>,
    /// First-paragraph body preview (≤280 chars, word-boundary truncated), from the
    /// R4 slice's `first_chunk` via `compute_excerpt`. `None` when the node has no
    /// body, or on any read that doesn't source a first chunk. Renders as the
    /// EXCERPT block in the TrailRail and the hover-card snippet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    /// Workflow stage (`backlog`/`in-progress`/`done`/`cancelled`) for doc-types that
    /// carry one — tasks, chiefly. `None` for every other doc-type and for reads that
    /// do not source it. Ported from the legacy subgraph's `stage_raw` (spec D8): stage
    /// is load-bearing on a builder surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// The id of the anchor this resource is homed in — **not** a decorated ref.
    ///
    /// `home` says which *kind*; this says which *one*. Deliberately an id: building `@owner/slug`
    /// server-side would mean a second copy of `graph_home_contexts`' owner_ref CASE, linked to the
    /// first by nothing. A client already holds every anchor it can read, with `slug` and
    /// `owner_ref` on each, so it resolves this locally and no expression is duplicated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_id: Option<Uuid>,
    /// When the resource last moved. Present so a node can carry its own recency without a
    /// second read — the orientation screen has no `ResourceView` behind its marks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<DateTime<Utc>>,
}

/// A directed edge on the Atlas canvas. `label` is nullable (matches
/// `kb_edges.label`), `weight` drives stroke thickness in the Atlas grammar.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasEdge {
    pub id: Uuid,
    pub source: Uuid,
    pub target: Uuid,
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub weight: f64,
}

/// The response body for an R4 neighborhood slice.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasSubgraph {
    pub nodes: Vec<AtlasNode>,
    pub edges: Vec<AtlasEdge>,
}

/// What the entry read was choosing from — so the surface can say *how many of how many*.
///
/// The bound line is deliberately **chrome, not a warning**: present whether or not the view is
/// partial, "so complete is something the reader is TOLD rather than something they infer from
/// silence" (spec §7.1). It is covered today only because the composition trace hands it these
/// numbers for free; the entry read runs no composition, so it must carry its own.
///
/// `in_scope - eligible` is the count of resources the read deliberately did **not** draw because
/// they have no visible connections. Naming it is what keeps
/// `legibility-is-never-bought-with-silent-omission` covered — on the corpus that produced this
/// design that difference is 1,077 resources, and dropping them unannounced is precisely the defect
/// the goal exists to prevent.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct EntryBounds {
    /// Marks actually returned.
    ///
    /// `i32` rather than `i64` deliberately: these numbers cross to a browser, and ts-rs maps a
    /// 64-bit count to `bigint`, which cannot survive `JSON.stringify` on the server/client
    /// boundary — the same type-fidelity trap that once stopped a correct composition leaving the
    /// process. `AtlasNode.degree` is already `i32`, so the payload stays one numeric kind.
    pub drawn: i32,
    /// How many resources cleared the connection floor — the denominator the drawn count is *of*.
    pub eligible: i32,
    /// Every resource visible to this reader within the places asked about, connected or not.
    pub in_scope: i32,
    /// Whether more eligible resources exist than were drawn.
    pub truncated: bool,
}

/// The entry read's response — an `AtlasSubgraph` that declares its own bounds.
///
/// Separate from [`AtlasSubgraph`] rather than a field added to it: every other graph read gets its
/// bound declaration from the composition trace that produced it, and giving them all an optional
/// bounds field would make "did anyone actually fill this in?" a runtime question. Here it is not
/// optional, because this read has no other source for it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasEntry {
    pub nodes: Vec<AtlasNode>,
    pub edges: Vec<AtlasEdge>,
    pub bounds: EntryBounds,
}

/// R4 request: focus seeds (required, non-empty), BFS depth, and an optional
/// edge-kind filter that constrains the *traversal* (induced subgraph).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct SliceRequest {
    /// Focus resource ids. Must be non-empty — R4 is always drilled in around a focus.
    pub seeds: Vec<Uuid>,
    /// BFS depth from the seed set. Clamped server-side to MAX_DEPTH (10).
    pub depth: u32,
    /// Edge-kind filter constraining the walk; empty = all kinds.
    #[serde(default)]
    pub edge_kinds: Vec<EdgeKind>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_node_doc_type_is_optional() {
        let n = AtlasNode {
            id: uuid::Uuid::nil(),
            title: "t".into(),
            doc_type: None,
            home: NodeHome::Cogmap,
            degree: 3,
            salience: Some(0.8),
            excerpt: None,
            stage: None,
            home_id: None,
            updated: None,
        };
        let json = serde_json::to_string(&n).unwrap();
        let back: AtlasNode = serde_json::from_str(&json).unwrap();
        assert_eq!(n, back);
        assert!(json.contains("\"home\":\"cogmap\""));
        assert!(!json.contains("doc_type")); // None is skipped
        assert!(!json.contains("excerpt")); // None is skipped
    }
}
