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
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::{EdgeId, ResourceId};
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
/// Compile-time-checked (`query!`), and the row is constructed explicitly rather
/// than decoded by `FromRow`: `GraphEdgeRow` carries the `EdgeId`/`ResourceId`
/// newtypes, which the macros do not decode into, so the ids come back as `Uuid`
/// and are converted in the mapping closure (the repo's incumbent shape).
pub async fn list_resource_edges(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<Vec<GraphEdgeRow>> {
    // 404 parity: an invisible/absent resource is NotFound (the gate runs before
    // listing, so a visible resource with no edges still returns Ok(empty)).
    // `visible!`: `EXISTS` yields TRUE or FALSE and never NULL, so the non-null override is safe
    // even though sqlx types every expression column as nullable.
    let visible: bool = sqlx::query_scalar!(
        r#"SELECT EXISTS (
            SELECT 1 FROM resources_visible_to($1) rv
             WHERE rv.resource_id = $2
        ) AS "visible!""#,
        profile_id,
        resource_id,
    )
    .fetch_one(pool)
    .await?;

    if !visible {
        return Err(ApiError::NotFound(
            "resource not found or not readable".to_string(),
        ));
    }

    // Nullability overrides, each earned: `peer_resource_id`/`direction` are total `CASE`s (an ELSE
    // arm, both arms non-null — `kb_edges.source_id`/`target_id` are NOT NULL); `peer_slug` composes
    // strict functions over `peer.title`, which is NOT NULL and INNER-joined; `label` is a COALESCE
    // onto a non-null literal. sqlx types every expression column nullable, which is why all four
    // need the annotation while the plain `e.*` columns do not.
    //
    // `edge_kind`/`polarity` take an explicit type override so the SQL enums decode straight into
    // `EdgeKind`/`Polarity` (both derive `sqlx::Type` with their `type_name`) — no `::text`
    // round-trip, matching what the runtime version decoded.
    let edges = sqlx::query!(
        r#"SELECT
            e.id AS edge_id,
            (CASE WHEN e.source_id = $2 THEN e.target_id ELSE e.source_id END) AS "peer_resource_id!",
            peer.title AS peer_title,
            lower(regexp_replace(
                regexp_replace(peer.title, '[^a-zA-Z0-9]+', '-', 'g'),
                '(^-+|-+$)', '', 'g')) AS "peer_slug!",
            e.edge_kind AS "edge_kind: EdgeKind",
            e.polarity AS "polarity: Polarity",
            COALESCE(e.label, '') AS "label!",
            (CASE WHEN e.source_id = $2 THEN 'outgoing' ELSE 'incoming' END) AS "direction!",
            e.weight AS weight,
            e.created AS created
          FROM kb_edges e
          JOIN edges_visible_to($1) v ON v.edge_id = e.id
          JOIN kb_resources peer
            ON peer.id = (CASE WHEN e.source_id = $2 THEN e.target_id ELSE e.source_id END)
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           AND (e.source_id = $2 OR e.target_id = $2)"#,
        profile_id,
        resource_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| GraphEdgeRow {
        edge_id: EdgeId::from(r.edge_id),
        peer_resource_id: ResourceId::from(r.peer_resource_id),
        peer_title: r.peer_title,
        peer_slug: r.peer_slug,
        edge_kind: r.edge_kind,
        polarity: r.polarity,
        label: r.label,
        direction: r.direction,
        weight: r.weight,
        created: r.created,
    })
    .collect();

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
/// Both queries here are compile-time-checked. The facet SELECT became a `query_as!` on 2026-07-29;
/// the visibility gate followed once the exemption it had been resting on was actually tested and
/// found false. That exemption claimed sqlx's compile-time describe could not resolve
/// `edges_visible_to`/`resources_visible_to` because their bodies reference helpers unqualified —
/// but the describe step never inlines a function body, and `team_service::is_visible` had been
/// calling `resources_visible_to` from a `query_scalar!` in this same crate the whole time.
///
/// Recorded rather than quietly corrected because the shape is worth recognising: an exemption
/// stated once at function scope silently covers every query added to that function afterwards, and
/// an unverified one spreads by citation — `lineage_service` adopted this exact claim by reference.
pub async fn list_edge_facets(
    pool: &PgPool,
    profile_id: Uuid,
    edge_id: Uuid,
) -> ApiResult<Vec<EdgeFacetRow>> {
    // `visible!`: `EXISTS` is never NULL (see the matching gate in `list_resource_edges`).
    let visible: bool = sqlx::query_scalar!(
        r#"SELECT EXISTS (
            SELECT 1 FROM edges_visible_to($1) v WHERE v.edge_id = $2
        ) AS "visible!""#,
        profile_id,
        edge_id,
    )
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
    // Selected straight into the wire type: the aliases ARE the mapping, so there is no positional
    // hand-off between an eight-slot tuple and an eight-field struct to get wrong. The three author
    // columns take `?` because they arrive through a LEFT JOIN, and sqlx infers nullability from the
    // column definition — `kb_profiles.handle` is NOT NULL, so without the annotation the macro would
    // type an absent author as a non-optional String.
    //
    // ORDER BY is unchanged from the runtime version. This is a form change, not a behaviour change.
    let rows = sqlx::query_as!(
        EdgeFacetRow,
        r#"
        SELECT p.id                    AS property_id,
               p.property_key          AS property_key,
               p.property_value        AS value,
               p.weight                AS weight,
               p.asserted_by_event_id  AS authored_by_event_id,
               pr.id                   AS "authored_by_profile_id?",
               pr.handle               AS "authored_by_handle?",
               pr.display_name         AS "authored_by_display_name?"
          FROM kb_properties p
          JOIN kb_events ev ON ev.id = p.asserted_by_event_id
          LEFT JOIN kb_entities en ON en.id = ev.emitter_entity_id
          LEFT JOIN kb_profiles pr ON pr.id = en.profile_id
         WHERE p.owner_table = 'kb_edges' AND p.owner_id = $1 AND NOT p.is_folded
         ORDER BY p.property_key, p.created
        "#,
        edge_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
