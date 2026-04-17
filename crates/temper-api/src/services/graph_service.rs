//! Graph subgraph service — returns aggregator-centric subgraphs for the
//! knowledge-graph UI.
//!
//! A "subgraph" is a depth-2 BFS from aggregator seeds (concepts today, any
//! aggregator doc type tomorrow). Composes with `graph_traverse()` for the
//! actual traversal so we inherit visibility scoping, cycle detection, and
//! edge-type filtering.

use sqlx::PgPool;
use sqlx::Row;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::frontmatter::document::DocType;
use temper_core::types::graph::{EdgeType, GraphEdge, GraphNode, SubgraphResponse};

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
/// Implementation uses three steps to avoid CTE duplication and close the
/// dangling-edge risk:
///
/// 1. Resolve the full active ID set once (seeds + traversed, both filtered
///    by `is_active = true`).
/// 2. Fetch node rows for those IDs.
/// 3. Fetch edge rows where both endpoints are in the ID set.
///
/// Because inactive resources are excluded at step 1, the edge query in
/// step 3 can never return a dangling edge.
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

    // Step 1: resolve the ID set once. The CTE returns seeds + traversed
    // nodes, then joins back to kb_resources with `is_active = true` to
    // guarantee only active rows end up in the set. Visibility is enforced
    // by resources_visible_to() in the seed CTE and by graph_traverse()
    // during expansion.
    let id_rows = sqlx::query(
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
            SELECT resource_id
              FROM graph_traverse(
                  $1,
                  ARRAY(SELECT id FROM seed_concepts),
                  $4::int,
                  '{}'
              )
        ),
        candidate_ids AS (
            SELECT id          FROM seed_concepts
            UNION
            SELECT resource_id FROM traversed
        )
        SELECT r.id AS id
          FROM kb_resources r
          JOIN candidate_ids c ON c.id = r.id
         WHERE r.is_active = true
        "#,
    )
    .bind(params.caller_profile_id)
    .bind(params.context_name)
    .bind(&aggregator_names)
    .bind(depth as i32)
    .fetch_all(pool)
    .await?;

    let node_ids: Vec<Uuid> = id_rows
        .into_iter()
        .map(|r| r.get::<Uuid, _>("id"))
        .collect();

    if node_ids.is_empty() {
        return Ok(SubgraphResponse {
            nodes: vec![],
            edges: vec![],
        });
    }

    // Step 2: node rows, bound directly to the resolved ID set.
    let node_rows = sqlx::query(
        r#"
        SELECT
            r.id                                                AS id,
            r.slug                                              AS slug,
            r.title                                             AS title,
            dt.name                                             AS doc_type,
            (SELECT COUNT(*)::int
               FROM kb_resource_edges e
              WHERE e.source_resource_id = r.id
                 OR e.target_resource_id = r.id)                AS edge_count
          FROM kb_resources r
          JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
         WHERE r.id = ANY($1::uuid[])
           AND r.is_active = true
        "#,
    )
    .bind(&node_ids)
    .fetch_all(pool)
    .await?;

    let nodes: Vec<GraphNode> = node_rows
        .into_iter()
        .map(|row| -> ApiResult<GraphNode> {
            let doc_type_str: String = row.get("doc_type");
            // DocType::from_str returns TemperError on unknown name; map to
            // ApiError::Internal since an unrecognised doctype is a data-integrity issue.
            let doc_type = DocType::from_str(&doc_type_str)
                .map_err(|e| ApiError::Internal(format!("unexpected doc_type in db: {e}")))?;
            Ok(GraphNode {
                id: row.get("id"),
                slug: row.get("slug"),
                title: row.get("title"),
                doc_type,
                edge_count: row.get("edge_count"),
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;

    // Step 3: edge rows — both endpoints must be in the resolved set.
    // Because node_ids only contains active resources (step 1), no dangling
    // edges can appear here.
    let edge_rows = sqlx::query(
        r#"
        SELECT
            e.source_resource_id AS source,
            e.target_resource_id AS target,
            e.edge_type          AS edge_type
          FROM kb_resource_edges e
         WHERE e.source_resource_id = ANY($1::uuid[])
           AND e.target_resource_id = ANY($1::uuid[])
        "#,
    )
    .bind(&node_ids)
    .fetch_all(pool)
    .await?;

    let edges: Vec<GraphEdge> = edge_rows
        .into_iter()
        .map(|row| GraphEdge {
            source: row.get("source"),
            target: row.get("target"),
            edge_type: row.get::<EdgeType, _>("edge_type"),
        })
        .collect();

    Ok(SubgraphResponse { nodes, edges })
}
