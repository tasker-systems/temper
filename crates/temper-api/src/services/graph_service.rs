//! Graph subgraph service — returns aggregator-centric subgraphs for the
//! knowledge-graph UI.
//!
//! A "subgraph" is a depth-2 BFS from aggregator seeds (concepts today, any
//! aggregator doc type tomorrow). Composes with `graph_traverse()` for the
//! actual traversal so we inherit visibility scoping, cycle detection, and
//! edge-type filtering.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use temper_core::frontmatter::document::DocType;
use temper_core::types::graph::SubgraphResponse;

/// Hard upper bound on traversal depth. Recursive-CTE cost grows superlinearly
/// with depth; 10 hops covers any imaginable UI traversal. Clamped silently.
const MAX_DEPTH: u32 = 10;

/// Parameters for `aggregator_subgraph`.
///
/// Factored into a struct so future filter additions (doc-type excludes,
/// edge-type filters) drop in without refactoring every call site.
#[derive(Debug, Clone)]
pub struct AggregatorSubgraphParams<'a> {
    pub caller_profile_id: Uuid,
    pub context_name: &'a str,
    pub aggregator_types: &'a [DocType],
    pub depth: u32,
}

/// Return a subgraph anchored on the given aggregator doc types within a
/// context, expanded by BFS to `depth` hops.
///
/// The caller's visibility is enforced by `graph_traverse()`'s internal
/// `resources_visible_to` join — cross-owner resources are never returned.
pub async fn aggregator_subgraph(
    pool: &PgPool,
    params: AggregatorSubgraphParams<'_>,
) -> ApiResult<SubgraphResponse> {
    // SAFETY CLAMP: unvalidated callers can't DoS Postgres with a runaway
    // recursive CTE. v1 hands us `2`, but the guard is cheap insurance.
    let depth = params.depth.min(MAX_DEPTH);

    // Implementation added in Task 4.
    let _ = (
        pool,
        params.caller_profile_id,
        params.context_name,
        params.aggregator_types,
        depth,
    );
    Ok(SubgraphResponse {
        nodes: vec![],
        edges: vec![],
    })
}
