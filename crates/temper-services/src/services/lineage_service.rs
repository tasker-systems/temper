//! Lineage service — Ledger L2's bidirectional `derived_from` reader.
//!
//! One read over the substrate graph: given a resource, what does it derive from
//! (ancestors) and what derives from it (descendants). The walk and its access
//! gate live in the SQL function `resource_lineage` (20260712000060), which
//! reuses the exact `element_trail_edge` visibility triple (home readable AND
//! both endpoints readable) and keys on the edge LABEL `derived_from` — spanning
//! both `edge_kind`s per L1's 2026-07-12 grounding.
//!
//! Runtime queries (not `query!` macros), matching `edge_service`: the SQL
//! function body references visibility helpers UNQUALIFIED, which sqlx's
//! compile-time describe cannot resolve against the build search_path. The result
//! rows decode into `LineageRow` by field name.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::ResourceId;
use temper_core::types::lineage::{LineageNode, ResourceLineage};
use temper_workflow::operations::decorated_ref;

/// One row from `resource_lineage(...)`. Column set is identical for both
/// directions.
#[derive(sqlx::FromRow)]
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
    let rows = sqlx::query_as::<_, LineageRow>(
        "SELECT resource_id, title, is_active, edge_id, edge_is_folded, depth
           FROM resource_lineage($1, $2, $3, $4)
          ORDER BY depth, title",
    )
    .bind(profile_id)
    .bind(resource_id)
    .bind(direction)
    .bind(max_depth)
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
    let visible: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM resources_visible_to($1) rv WHERE rv.resource_id = $2
        )",
    )
    .bind(profile_id)
    .bind(resource_id)
    .fetch_one(pool)
    .await?;

    if !visible {
        return Err(ApiError::NotFound);
    }

    let ancestors = walk(pool, profile_id, resource_id, "ancestors", max_depth).await?;
    let descendants = walk(pool, profile_id, resource_id, "descendants", max_depth).await?;

    Ok(ResourceLineage {
        resource_id,
        ancestors,
        descendants,
    })
}
