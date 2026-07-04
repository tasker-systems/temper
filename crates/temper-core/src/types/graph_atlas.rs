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
}

/// A directed edge on the Atlas canvas. `label` is nullable (matches
/// `kb_edges.label`), `weight` drives stroke thickness in the Atlas grammar.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasEdge {
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

/// A team-scoped search hit on the Atlas canvas. `node_id` is `kb_resources.id`
/// (identical to `AtlasNode.id`, so the UI can drill straight to it). Scores are
/// the `unified_search` blend, inherited verbatim. `region_id` is a best-affinity
/// territory hint (may be `None`); the camera jump uses `node_id` alone.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasSearchHit {
    pub node_id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    pub home: NodeHome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_id: Option<Uuid>,
    pub combined_score: f32,
    pub fts_score: f32,
    pub vector_score: f32,
    pub graph_score: f32,
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
        };
        let json = serde_json::to_string(&n).unwrap();
        let back: AtlasNode = serde_json::from_str(&json).unwrap();
        assert_eq!(n, back);
        assert!(json.contains("\"home\":\"cogmap\""));
        assert!(!json.contains("doc_type")); // None is skipped
    }
}
