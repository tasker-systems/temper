//! Graph subgraph service — returns aggregator-centric subgraphs for the
//! knowledge-graph UI.
//!
//! A "subgraph" is a depth-2 BFS from aggregator seeds (concepts today, any
//! aggregator doc type tomorrow). Composes with `graph_traverse()` for the
//! actual traversal so we inherit visibility scoping, cycle detection, and
//! edge-type filtering.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::frontmatter::document::DocType;
use temper_core::types::graph::{is_aggregator, EdgeType, GraphEdge, GraphNode, SubgraphResponse};

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
///
/// **Sessions are not nodes.** Per the R11 visual language (sessions are
/// annotations, not graph participants), session-typed resources are filtered
/// out of the returned node set. Each remaining node carries a
/// `session_count` equal to the number of sessions that share an edge with
/// it. Edges whose endpoint is a session are likewise dropped.
///
/// Implementation uses two round-trips with compile-time checked queries:
///
/// 1. CTE-resolved node rows (seeds + traversed, both filtered by
///    `is_active = true` AND `doc_type != 'session'`) — node metadata plus
///    the per-node session count come back in the same trip.
/// 2. Edge rows where both endpoints are in the resolved (non-session) ID set.
///
/// Because inactive and session resources are excluded at step 1, the edge
/// query in step 2 can never return a dangling or session-incident edge.
pub async fn aggregator_subgraph(
    pool: &PgPool,
    params: AggregatorSubgraphParams<'_>,
) -> ApiResult<SubgraphResponse> {
    // SAFETY CLAMP: unvalidated callers can't DoS Postgres with a runaway
    // recursive CTE. v1 hands us `2`, but the guard is cheap insurance.
    let depth = params.depth.min(MAX_DEPTH);

    // DocType → lowercase name for the kb_doc_types.name match.
    let aggregator_names: Vec<String> = params
        .aggregator_types
        .iter()
        .map(|dt| dt.as_str().to_string())
        .collect();

    // Query 1: resolve the candidate ID set AND fetch node metadata in one
    // round-trip. The CTE unions seeds + traversed, then joins back to
    // kb_resources with `is_active = true` AND `doc_type != 'session'` to
    // exclude sessions from the node list. Visibility is enforced by
    // resources_visible_to() in the seed CTE and by graph_traverse() during
    // expansion.
    //
    // session_count is computed as a correlated subquery: for each returned
    // node, how many session-typed resources share any edge with it?
    let node_records = sqlx::query!(
        r#"
        WITH seed_concepts AS (
            SELECT r.id
              FROM kb_resources r
              JOIN resources_visible_to($1, NULL, '{}') v ON v.resource_id = r.id
              JOIN kb_contexts c   ON c.id  = r.kb_context_id
              JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
             WHERE c.name = $2
               AND dt.name = ANY($3::text[])
               AND r.is_active = true
        ),
        traversed AS (
            SELECT resource_id AS id
              FROM graph_traverse(
                  $1,
                  ARRAY(SELECT id FROM seed_concepts),
                  $4::int,
                  '{}'
              )
        ),
        candidate_ids AS (
            SELECT id FROM seed_concepts
            UNION
            SELECT id FROM traversed
        )
        SELECT
            r.id         AS "id!: Uuid",
            r.slug       AS "slug!",
            r.title      AS "title!",
            dt.name      AS "doc_type!",
            (SELECT COUNT(*)::int
               FROM kb_resource_edges e
              WHERE e.source_resource_id = r.id
                 OR e.target_resource_id = r.id) AS "edge_count!: i32",
            (SELECT COUNT(DISTINCT peer.id)::int
               FROM kb_resource_edges e
               JOIN kb_resources peer
                 ON peer.id = CASE WHEN e.source_resource_id = r.id
                                   THEN e.target_resource_id
                                   ELSE e.source_resource_id
                              END
               JOIN kb_doc_types peer_dt ON peer_dt.id = peer.kb_doc_type_id
              WHERE (e.source_resource_id = r.id OR e.target_resource_id = r.id)
                AND peer_dt.name = 'session'
                AND peer.is_active = true) AS "session_count!: i32"
          FROM kb_resources r
          JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
          JOIN candidate_ids c ON c.id = r.id
         WHERE r.is_active = true
           AND dt.name <> 'session'
        "#,
        params.caller_profile_id,
        params.context_name,
        &aggregator_names,
        depth as i32,
    )
    .fetch_all(pool)
    .await?;

    if node_records.is_empty() {
        return Ok(SubgraphResponse {
            nodes: vec![],
            edges: vec![],
        });
    }

    let mut node_ids: Vec<Uuid> = Vec::with_capacity(node_records.len());
    let mut nodes: Vec<GraphNode> = Vec::with_capacity(node_records.len());
    for rec in node_records {
        // DocType::from_str returns TemperError on unknown name; map to
        // ApiError::Internal since an unrecognised doctype is a data-integrity issue.
        let doc_type = DocType::from_str(&rec.doc_type)
            .map_err(|e| ApiError::Internal(format!("unexpected doc_type in db: {e}")))?;
        node_ids.push(rec.id);
        nodes.push(GraphNode {
            id: rec.id,
            slug: rec.slug,
            title: rec.title,
            aggregator: is_aggregator(doc_type),
            doc_type,
            edge_count: rec.edge_count,
            session_count: rec.session_count,
        });
    }

    // Query 2: edge rows — both endpoints must be in the resolved set.
    // Because node_ids only contains active resources (query 1), no dangling
    // edges can appear here.
    let edge_records = sqlx::query!(
        r#"
        SELECT
            source_resource_id AS "source!: Uuid",
            target_resource_id AS "target!: Uuid",
            edge_type          AS "edge_type!: EdgeType"
          FROM kb_resource_edges
         WHERE source_resource_id = ANY($1::uuid[])
           AND target_resource_id = ANY($1::uuid[])
        "#,
        &node_ids,
    )
    .fetch_all(pool)
    .await?;

    let edges: Vec<GraphEdge> = edge_records
        .into_iter()
        .map(|rec| GraphEdge {
            source: rec.source,
            target: rec.target,
            edge_type: rec.edge_type,
        })
        .collect();

    Ok(SubgraphResponse { nodes, edges })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_depth_constant_is_ten() {
        assert_eq!(MAX_DEPTH, 10);
    }

    #[test]
    fn depth_within_limit_passes_through() {
        // Compile-check: clamp is `params.depth.min(MAX_DEPTH)`.
        // We unit-test the clamp arithmetic; integration tests cover end-to-end.
        assert_eq!(5u32.min(MAX_DEPTH), 5);
    }

    #[test]
    fn depth_over_limit_clamps_to_max() {
        assert_eq!(100u32.min(MAX_DEPTH), 10);
        // u32::MAX is trivially >= MAX_DEPTH by type; assert via the ordering
        // the clamp relies on, without triggering `clippy::unnecessary_min_or_max`.
        assert!(u32::MAX > MAX_DEPTH);
    }
}
