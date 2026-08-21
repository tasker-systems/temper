//! Beat E — the context door's reads. Persistence lives in SQL
//! (`20260709000010_graph_context_reads.sql`, `20260709000011_atlas_nodes_visible_stage.sql`);
//! this module composes those functions into the panorama + composition wire shapes.
//! No `sqlx::query!()` ever appears in a surface — HTTP handlers and MCP tools call these
//! functions, which own every query.
//!
//! Every read scopes through `resources_visible_to($profile)` (or `anchor_readable_by_profile`
//! for edges) inside the SQL functions themselves — deny-as-absence: an invisible resource
//! is simply absent from the result, never a leaked count or a forbidden-but-exists signal.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::graph_atlas::AtlasSubgraph;
use temper_core::types::graph_context::{
    ContextPanorama, GroupKeyMeta, ResidualBucket, ResidualGroups,
};
use temper_core::types::graph_territory::{Territory, TerritoryKind};
use temper_core::types::ids::{ContextId, ProfileId};

use crate::error::ApiResult;

/// Tier-0 of the context door: goal-rooted container territories plus the residual tray.
///
/// `container_types` defaults (at the surface) to `["goal"]` and `group_key` to `"doc_type"`,
/// but both are parameters (spec D4/D2) — nothing here is hard-coded to `goal`/`session`.
pub async fn context_panorama(
    pool: &PgPool,
    profile_id: ProfileId,
    context_id: ContextId,
    group_key: &str,
    container_types: &[String],
    depth: i32,
) -> ApiResult<ContextPanorama> {
    // `id!`: the body's `containers` CTE reads `c.id` out of `ctx`, which is
    // `JOIN kb_resources r` — `kb_resources.id` is the PK. `member_count!`:
    // `(SELECT count(DISTINCT rr.node_id)::int - 1 FROM reached rr WHERE rr.root = c.id)` — a
    // scalar aggregate subquery, so always a row, and `count() - 1` of a non-null is non-null.
    // `label` is `c.title`, which traces to the NOT NULL `kb_resources.title`, but it is left
    // UNANNOTATED on purpose: `Territory.label` is `Option<String>` because its other producer
    // (`graph_cogmap_territories`) genuinely returns NULL there, and passing this one straight
    // through keeps both producers filling the field the same way. An absent `!` is never a
    // decode hazard; only a wrong one is.
    let containers: Vec<Territory> = sqlx::query!(
        r#"SELECT id AS "id!", label, member_count AS "member_count!"
             FROM graph_context_containers($1, $2, $3, $4)"#,
        profile_id.as_uuid(),
        *context_id,
        container_types,
        depth,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|c| Territory {
        id: c.id,
        // Tint encodes the AXIS, not container-ness (spec D6). A goal container sits on
        // the builder axis, so it is Context-tinted even though it is rooted at a goal.
        kind: TerritoryKind::Context,
        label: c.label,
        member_count: c.member_count,
        salience: None,
        coherence: None,
        anchor_id: *context_id,
    })
    .collect();

    // Both columns take `!` only because a set-returning function's columns always read as nullable.
    // In the body (`\sf graph_context_residual_counts`) `group_value` is `COALESCE(g.gv, '(none)')`
    // — a COALESCE onto a non-null literal — and `member_count` is `count(*)::int`, which is 0 over
    // an empty group, never NULL.
    let buckets: Vec<ResidualBucket> = sqlx::query!(
        r#"SELECT group_value AS "group_value!", member_count AS "member_count!"
             FROM graph_context_residual_counts($1, $2, $3, $4, $5)"#,
        profile_id.as_uuid(),
        *context_id,
        group_key,
        container_types,
        depth,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| ResidualBucket {
        value: r.group_value,
        count: r.member_count,
    })
    .collect();

    let group_keys = available_group_keys(pool, profile_id, context_id).await?;

    Ok(ContextPanorama {
        containers,
        residual: ResidualGroups {
            group_key: group_key.to_string(),
            buckets,
        },
        group_keys,
    })
}

/// What else the caller could group the residual tray by, and how much of the context each
/// candidate key covers. Visibility-scoped: only keys carried by resources the profile can
/// see are considered, so this leaks nothing about resources outside the profile's reach.
async fn available_group_keys(
    pool: &PgPool,
    profile_id: ProfileId,
    context_id: ContextId,
) -> ApiResult<Vec<GroupKeyMeta>> {
    // The two counts take `!` because sqlx types every expression column as nullable; `count()` is
    // never NULL, and the `::int` cast of a non-null value stays non-null. `property_key` is a plain
    // column reference (`kb_properties.property_key` is NOT NULL), so it needs no annotation.
    Ok(sqlx::query!(
        r#"
        SELECT p.property_key,
               count(DISTINCT p.property_value #>> '{}')::int AS "distinct_values!",
               count(DISTINCT p.owner_id)::int               AS "coverage!"
          FROM kb_properties p
          JOIN kb_resource_homes h ON h.resource_id = p.owner_id
                                  AND h.anchor_table = 'kb_contexts' AND h.anchor_id = $2
          JOIN resources_visible_to($1) v ON v.resource_id = p.owner_id
         WHERE p.owner_table = 'kb_resources' AND NOT p.is_folded
         GROUP BY 1 HAVING count(DISTINCT p.property_value #>> '{}') BETWEEN 2 AND 24
         ORDER BY 3 DESC
        "#,
        profile_id.as_uuid(),
        *context_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| GroupKeyMeta {
        key: r.property_key,
        distinct_values: r.distinct_values,
        coverage: r.coverage,
    })
    .collect())
}

/// Identifies one residual bucket to drill: the context, the group key/value that defines
/// the bucket, and the container-walk parameters that decide which resources count as
/// "already contained" (and are therefore excluded from the bucket).
///
/// A params struct because this carries six domain values — over the five-parameter
/// threshold that warrants grouping them into a named type.
#[derive(Debug, Clone)]
pub struct ResidualMemberQuery<'a> {
    pub profile_id: ProfileId,
    pub context_id: ContextId,
    pub group_key: &'a str,
    pub group_value: &'a str,
    pub container_types: &'a [String],
    pub depth: i32,
}

/// The resource ids behind one residual bucket — the seeds a bucket drill feeds into
/// [`context_composition`]. Visibility-scoped in SQL (deny-as-absence).
pub async fn residual_member_ids(
    pool: &PgPool,
    query: ResidualMemberQuery<'_>,
) -> ApiResult<Vec<Uuid>> {
    // `id!`: nullable only because it comes from a set-returning function — the body projects
    // `c.id`, which traces back to `kb_resources.id` (the primary key).
    Ok(sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM graph_context_residual_members($1, $2, $3, $4, $5, $6)"#,
        query.profile_id.as_uuid(),
        *query.context_id,
        query.group_key,
        query.group_value,
        query.container_types,
        query.depth,
    )
    .fetch_all(pool)
    .await?)
}

/// Tier-1 of the context door: the force-graph composition of a container's (or bucket's)
/// members. Given the drill's seed set, walks visible edges out to `depth` hops — NOT fenced
/// to the context (spec: the walk follows visible edges out to cogmap-homed resources, which
/// is what makes "the work + the ideas distilled from it" one graph).
///
/// **This is now a thin alias over [`crate::services::graph_service::traversal_slice`].** The two had identical
/// bodies — seeds → induced edges → node set → hydrate → drop dangling — and the grounding/navigation
/// split named that body the *traversal read* (chunk B) rather than anything context-shaped. Two
/// copies of one walk linked by nothing is the drift this repo has already ruled against, so the
/// context door borrows the frame-neutral one instead of keeping its own.
///
/// The context door itself — this function, its handler and `context_panorama` — is deleted in
/// chunk E. `traversal_slice` is what survives.
pub async fn context_composition(
    pool: &PgPool,
    profile_id: ProfileId,
    seeds: &[Uuid],
    depth: i32,
) -> ApiResult<AtlasSubgraph> {
    crate::services::graph_service::traversal_slice(pool, profile_id, seeds, depth).await
}
