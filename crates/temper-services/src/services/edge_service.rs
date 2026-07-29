//! Edge service — the one live read over the substrate graph.
//!
//! Frontmatter→edge derivation (extract/reconcile/project) was retired with the
//! flip (product decision 1); edge writes route through the backend's
//! relationship commands (`02_functions.sql`). What remains here is the single
//! read the `/api/resources/{id}/edges` handler needs.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::facet_requests::EdgeFacetRow;
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
        return Err(ApiError::NotFound(
            "resource not found or not readable".to_string(),
        ));
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

/// List the live properties owned by one edge, scoped to profile visibility.
///
/// The read gate is `edges_visible_to` — the same predicate `list_resource_edges` applies and the
/// same one the edge-authorship clauses answer to. Reading an edge's facets is not a wider
/// disclosure than reading the edge: the facet qualifies a link the caller can already see.
/// An edge that is absent *or* invisible is `NotFound`, so the endpoint never becomes an existence
/// oracle for edges in contexts the caller cannot read.
///
/// Folded rows are excluded. Since folding an edge cascades to the properties it owns
/// (`_project_relationship_folded`), a live row here always belongs to a live edge.
///
/// **A folded edge is `NotFound`, not an empty list** — `edges_visible_to` is `WHERE NOT
/// e.is_folded`, so a retracted relationship is invisible and so are its facets. That falls out of
/// the incumbent predicate rather than being a rule for facets, which is why it is not special-cased
/// here.
///
/// Runtime query rather than `query_as!` for the reason documented on [`list_resource_edges`]:
/// the visibility helpers are SQL functions that reference other helpers unqualified, which
/// sqlx's compile-time describe cannot resolve.
pub async fn list_edge_facets(
    pool: &PgPool,
    profile_id: Uuid,
    edge_id: Uuid,
) -> ApiResult<Vec<EdgeFacetRow>> {
    let visible: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM edges_visible_to($1) v WHERE v.edge_id = $2
        )",
    )
    .bind(profile_id)
    .bind(edge_id)
    .fetch_one(pool)
    .await?;

    if !visible {
        return Err(ApiError::NotFound(
            "edge not found or not readable".to_string(),
        ));
    }

    // Attribution is joined here rather than left to the caller: the edge's own trail cannot
    // recover it. `element_trail_edge` joins events on `payload->>'edge_id'`, and a
    // `property_asserted` payload carries `owner.{table,id}` instead — so the one surface built to
    // answer "what happened to this edge" is structurally blind to its facets. Until that is
    // reconciled, this read is the only place an author is recoverable, which is why it is not
    // optional here.
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            serde_json::Value,
            f64,
            Uuid,
            Option<Uuid>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT p.id, p.property_key, p.property_value, p.weight,
                p.asserted_by_event_id, pr.id, pr.handle, pr.display_name
           FROM kb_properties p
           JOIN kb_events ev ON ev.id = p.asserted_by_event_id
           LEFT JOIN kb_entities en ON en.id = ev.emitter_entity_id
           LEFT JOIN kb_profiles pr ON pr.id = en.profile_id
          WHERE p.owner_table = 'kb_edges' AND p.owner_id = $1 AND NOT p.is_folded
          ORDER BY p.property_key, p.created",
    )
    .bind(edge_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                property_id,
                property_key,
                value,
                weight,
                authored_by_event_id,
                authored_by_profile_id,
                authored_by_handle,
                authored_by_display_name,
            )| EdgeFacetRow {
                property_id,
                property_key,
                value,
                weight,
                authored_by_event_id,
                authored_by_profile_id,
                authored_by_handle,
                authored_by_display_name,
            },
        )
        .collect())
}
