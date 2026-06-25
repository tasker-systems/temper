//! Edge service — the one live read over the substrate graph.
//!
//! Frontmatter→edge derivation (extract/reconcile/project) was retired with the
//! flip (product decision 1); edge writes route through the backend's
//! relationship commands (`02_functions.sql`). What remains here is the single
//! read the `/api/resources/{id}/edges` handler needs.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_workflow::types::graph::GraphEdgeRow;

/// List the edges incident to a resource, scoped to profile visibility.
///
/// Reads the substrate `kb_edges` + `edges_visible_to`. Returns
/// [`GraphEdgeRow`] for the `/edges` handler. §9-non-invariant shaping:
/// - `peer_slug` is §7-dissolved in the substrate, so it is derived from the
///   peer title (matching Rust `text::slugify` / the substrate `graph_nodes`).
/// - `direction` keeps the legacy `'outgoing'`/`'incoming'` vocabulary, derived
///   from which endpoint is the queried resource.
///
/// Uses RUNTIME queries (not `query!` macros): sqlx's compile-time describe
/// inlines the SQL-function bodies at plan time; `resources_visible_to` /
/// `edges_visible_to` reference helpers UNQUALIFIED, which the describe step
/// resolves against the build connection's search_path. Keeping these runtime
/// sidesteps that. The result row decodes into the `sqlx::FromRow`-deriving
/// `GraphEdgeRow` by field name; `COALESCE(label, '')` fills the nullable label.
pub async fn list_resource_edges(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<Vec<GraphEdgeRow>> {
    // 404 parity: an invisible/absent resource is NotFound (the gate runs before
    // listing, so a visible resource with no edges still returns Ok(empty)).
    let visible: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM resources_visible_to($1) rv
             WHERE rv.resource_id = $2
        )",
    )
    .bind(profile_id)
    .bind(resource_id)
    .fetch_one(pool)
    .await?;

    if !visible {
        return Err(ApiError::NotFound);
    }

    let edges = sqlx::query_as::<_, GraphEdgeRow>(
        "SELECT
            e.id AS edge_id,
            (CASE WHEN e.source_id = $2 THEN e.target_id ELSE e.source_id END) AS peer_resource_id,
            peer.title AS peer_title,
            lower(regexp_replace(
                regexp_replace(peer.title, '[^a-zA-Z0-9]+', '-', 'g'),
                '(^-+|-+$)', '', 'g')) AS peer_slug,
            e.edge_kind AS edge_kind,
            e.polarity AS polarity,
            COALESCE(e.label, '') AS label,
            (CASE WHEN e.source_id = $2 THEN 'outgoing' ELSE 'incoming' END) AS direction,
            e.weight AS weight,
            e.created AS created
          FROM kb_edges e
          JOIN edges_visible_to($1) v ON v.edge_id = e.id
          JOIN kb_resources peer
            ON peer.id = (CASE WHEN e.source_id = $2 THEN e.target_id ELSE e.source_id END)
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           AND (e.source_id = $2 OR e.target_id = $2)",
    )
    .bind(profile_id)
    .bind(resource_id)
    .fetch_all(pool)
    .await?;

    Ok(edges)
}
