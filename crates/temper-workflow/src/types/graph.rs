//! Knowledge graph types — edge declarations and traversal/neighbor result rows
//! for the R7 vertex-edge graph stored in `kb_resource_edges`.
//!
//! The neutral structural taxonomy (`EdgeKind`/`Polarity`) lives in
//! `temper_core::types::graph`; the frontmatter-relation mapping half lives here.

use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::ResourceId;

// The graph's wire shapes (and `TargetRef`, which `ResourceRelationships::to_edge_declarations`
// returns) moved to `temper_core::types::graph`: every wire type is temper-core's. Re-exported so
// each `temper_workflow::types::graph::…` call site resolves unchanged.
pub use temper_core::types::graph::{
    EdgeType, GraphEdgeRow, GraphNeighborRow, GraphTraversalRow, ResourceConnections,
    ResourceRelationships, TargetRef,
};

/// A resolved edge ready for projection.
#[derive(Debug, Clone)]
pub struct ResolvedEdge {
    pub source_resource_id: ResourceId,
    pub target_resource_id: ResourceId,
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    pub label: String,
    pub weight: f64,
}

/// Result of edge reconciliation after a frontmatter update.
///
/// - `added`     — new `relationship_asserted` events whose target resolved.
/// - `removed`   — projected edges retracted via `relationship_folded`.
/// - `unchanged` — declarations already present in the projection.
/// - `pending`   — `relationship_asserted` events whose target is still an
///   unresolved slug; the projection will materialize when the target lands.
#[derive(Debug, Clone)]
pub struct EdgeReconciliation {
    pub added: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub pending: usize,
}
