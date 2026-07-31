//! Lineage service — Ledger L2's bidirectional `derived_from` reader.
//!
//! One read over the substrate graph: given a resource, what does it derive from
//! (ancestors) and what derives from it (descendants). The walk and its access
//! gate live in the SQL function `resource_lineage` (20260712000080), which
//! reuses the exact `element_trail_edge` visibility triple (home readable AND
//! both endpoints readable) and keys on the edge LABEL `derived_from` — spanning
//! both `edge_kind`s per L1's 2026-07-12 grounding.
//!
//! Compile-time-checked (`query!` macros). These were runtime, citing `edge_service`'s claim that
//! sqlx's compile-time describe cannot resolve the visibility helpers a SQL function body
//! references unqualified. That claim was never true — describe does not inline a function body —
//! and both files are macros now.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::ResourceId;
use temper_core::types::lineage::{LineageNode, ResourceLineage};
use temper_workflow::operations::decorated_ref;

/// One row from `resource_lineage(...)`. Column set is identical for both
/// directions.
///
/// No `FromRow` derive: `query_as!` constructs positionally and cannot use one, so the field order
/// here IS the mapping and must match the SELECT list in [`walk`].
struct LineageRow {
    resource_id: Uuid,
    title: String,
    is_active: bool,
    edge_id: Uuid,
    edge_is_folded: bool,
    depth: i32,
}

impl From<LineageRow> for LineageNode {
    fn from(r: LineageRow) -> Self {
        LineageNode {
            r#ref: decorated_ref(&r.title, ResourceId::from(r.resource_id)),
            resource_id: r.resource_id,
            title: r.title,
            is_active: r.is_active,
            edge_id: r.edge_id,
            edge_folded: r.edge_is_folded,
            depth: r.depth,
        }
    }
}

/// Walk one direction of a resource's `derived_from` lineage, access-gated.
/// `direction` is `'ancestors'` or `'descendants'` (validated at the call site,
/// never caller-supplied text).
async fn walk(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
    direction: &str,
    max_depth: i32,
) -> ApiResult<Vec<LineageNode>> {
    // Every column takes `!`: sqlx types all six as nullable purely because they come from a
    // set-returning function, but the function's final SELECT (`\sf resource_lineage`) projects
    // `r.title`/`r.is_active` across an INNER `JOIN kb_resources` (both NOT NULL) and
    // `w.resource_id`/`w.edge_id`/`w.edge_is_folded` off `kb_edges` columns that are NOT NULL,
    // with `depth` a literal-seeded counter. None of the six can be NULL.
    let rows = sqlx::query_as!(
        LineageRow,
        r#"SELECT resource_id AS "resource_id!",
                  title AS "title!",
                  is_active AS "is_active!",
                  edge_id AS "edge_id!",
                  edge_is_folded AS "edge_is_folded!",
                  depth AS "depth!"
           FROM resource_lineage($1, $2, $3, $4)
          ORDER BY depth, title"#,
        profile_id,
        resource_id,
        direction,
        max_depth,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(LineageNode::from).collect())
}

/// The seed's bidirectional `derived_from` lineage, each side gated to what the
/// profile may read. An invisible/absent seed is `NotFound` (404 parity with
/// `list_resource_edges`) — the gate runs before the walk, so a visible resource
/// with no lineage returns empty sides.
pub async fn resource_lineage(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
    max_depth: i32,
) -> ApiResult<ResourceLineage> {
    // `visible!`: `EXISTS` is never NULL.
    let visible: bool = sqlx::query_scalar!(
        r#"SELECT EXISTS (
            SELECT 1 FROM resources_visible_to($1) rv WHERE rv.resource_id = $2
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

    let ancestors = walk(pool, profile_id, resource_id, "ancestors", max_depth).await?;
    let descendants = walk(pool, profile_id, resource_id, "descendants", max_depth).await?;

    Ok(ResourceLineage {
        resource_id,
        ancestors,
        descendants,
    })
}
