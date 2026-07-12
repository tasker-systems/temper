//! Ledger L2 — the derived-from lineage of a resource: what it derives from
//! (ancestors) and what derives from it (descendants), access-gated.
//!
//! Lineage lives on `derived_from` EDGES, not on `kb_events.references` (L1's
//! 2026-07-12 decision — references is write-dead in prod). The walk keys on the
//! edge LABEL `derived_from`, which is projected under two `edge_kind`s
//! (`express`/`leads_to`); keying on `edge_kind` would drop ~2/3 of the graph.
//! Each reached node carries the edge it was reached by and whether that edge is
//! folded — a folded ancestor is *shown*, flagged, because "you rest on a
//! superseded ancestor" is the point of the read.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One resource reached while walking a seed's `derived_from` lineage.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "lineage.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct LineageNode {
    pub resource_id: Uuid,
    /// Decorated ref (`sluggify(title)-<uuid>`) — paste-able into any ref-taking command.
    pub r#ref: String,
    pub title: String,
    /// The resource's soft-delete state (`kb_resources.is_active`).
    pub is_active: bool,
    /// The `derived_from` edge by which this node was reached from the frontier.
    pub edge_id: Uuid,
    /// Whether the reaching edge is folded — the supersession signal at read time.
    pub edge_folded: bool,
    /// Hop distance from the seed (1 = a direct neighbour); the shallowest reach.
    pub depth: i32,
}

/// A resource's bidirectional `derived_from` lineage, each side already gated to
/// what the caller may read.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "lineage.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceLineage {
    /// The seed resource the lineage was requested for.
    pub resource_id: Uuid,
    /// What the seed derives from, transitively (nearest first).
    pub ancestors: Vec<LineageNode>,
    /// What derives from the seed, transitively (nearest first).
    pub descendants: Vec<LineageNode>,
}
