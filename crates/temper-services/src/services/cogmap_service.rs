//! Cognitive-map ↔ team binding service over the substrate.
//!
//! Service-direct, NO Backend-trait command, NO event emission — a team↔cogmap
//! binding is provisioning/infrastructure, exactly like team membership
//! (`team_service`), not knowledge-graph content (org-provisioning spec §2.6, the
//! same precedent as `context_service`). Only cogmap *genesis* got a Backend
//! command.
//!
//! Bind/unbind gate is TWO-SIDED (at the TOP of each fn, BEFORE any write): `is_system_admin`, OR
//! the caller administers the MAP (`can_grant` on it) AND may manage the TEAM (`can_manage` =
//! owner/maintainer, direct membership) AND the team is NOT the gating/root team.
//!
//! That policy is no longer written here. It is `crate::authz::TwoSidedAuthority`, shared with
//! `context_service`'s share/unshare/reassign — the two were the same gate twice, differing only in
//! how authority over the object is established. **The gating-team exclusion cuts both ways and the
//! UNBIND direction is the load-bearing one**; the reason lives on that impl's `resolve`, since
//! that is where a future reader would go to relax it.

use sqlx::PgPool;
use uuid::Uuid;

use crate::authz::{TwoSidedAuthority, TwoSidedScope};
use crate::error::{ApiError, ApiResult};
use temper_core::types::cognitive_maps::{
    BindTeamOutcome, BindTeamRequest, CogmapDetail, CogmapFoundationRow, CogmapRow,
    UnbindTeamOutcome,
};
use temper_core::types::ids::ProfileId;

/// List every cognitive map visible to the profile, with identity + charter statement.
///
/// No entry gate: the read is self-scoped inside `cogmap_list_rows` via `cogmap_visible_maps`
/// (up-expanded team membership ∪ explicit read grant), so it returns exactly the maps the caller
/// may see — deny is an empty list, never an error. The charter statement rides the same
/// member-gated `resource_blocks` projection the charter read uses. Mirrors `context_service::
/// list_visible` in shape.
pub async fn list_visible(pool: &PgPool, profile_id: ProfileId) -> ApiResult<Vec<CogmapRow>> {
    let rows = sqlx::query_as!(
        CogmapRow,
        r#"
        SELECT cogmap_id            AS "id!",
               name                 AS "name!",
               owner_ref            AS "owner_ref!",
               team_ids             AS "team_ids!",
               region_count         AS "region_count!",
               resource_count       AS "resource_count!",
               telos_resource_id    AS "telos_resource_id!",
               charter_statement
          FROM cogmap_list_rows($1, NULL::uuid)
        "#,
        profile_id.uuid()
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// One map's full orientation in a single read: identity + charter blocks + foundational resources.
///
/// Map-read gated: the identity query passes the map id into `cogmap_list_rows`, which is scoped by
/// `cogmap_visible_maps` — an unreadable map yields zero rows, surfaced here as `NotFound` (never a
/// partial leak). The charter reuses `cogmap_charter_select` (member-gated blocks), and the
/// foundations are the map's visible homed resources with the telos flagged (`cogmap_foundations`).
pub async fn show_visible(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: Uuid,
) -> ApiResult<CogmapDetail> {
    let cogmap = sqlx::query_as!(
        CogmapRow,
        r#"
        SELECT cogmap_id            AS "id!",
               name                 AS "name!",
               owner_ref            AS "owner_ref!",
               team_ids             AS "team_ids!",
               region_count         AS "region_count!",
               resource_count       AS "resource_count!",
               telos_resource_id    AS "telos_resource_id!",
               charter_statement
          FROM cogmap_list_rows($1, $2)
        "#,
        profile_id.uuid(),
        cogmap_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("cognitive map not found or not readable".to_string()))?;

    let charter =
        crate::backend::substrate_read::cogmap_charter_select(pool, profile_id, cogmap_id).await?;

    let foundations = sqlx::query_as!(
        CogmapFoundationRow,
        r#"
        SELECT resource_id  AS "resource_id!",
               title        AS "title!",
               doc_type     AS "doc_type!",
               is_telos     AS "is_telos!"
          FROM cogmap_foundations($1, $2)
        "#,
        profile_id.uuid(),
        cogmap_id
    )
    .fetch_all(pool)
    .await?;

    Ok(CogmapDetail {
        cogmap,
        charter,
        foundations,
    })
}

/// Bind a cognitive map to a team (write a `kb_team_cogmaps` row).
///
/// Auth before writes: admin-only. Idempotent — `INSERT … ON CONFLICT DO NOTHING`;
/// `bound: false` when the binding already existed.
pub async fn bind_team(
    pool: &PgPool,
    caller: ProfileId,
    cogmap_id: Uuid,
    req: &BindTeamRequest,
) -> ApiResult<BindTeamOutcome> {
    // Auth before writes: system-admin, OR a team manager who administers the map (non-root team).
    crate::authz::authorize::<TwoSidedAuthority>(
        pool,
        caller,
        TwoSidedScope::cogmap(cogmap_id, req.team_id),
    )
    .await?;

    let inserted = sqlx::query_scalar!(
        r#"
        INSERT INTO kb_team_cogmaps (cogmap_id, team_id)
        VALUES ($1, $2)
        ON CONFLICT DO NOTHING
        RETURNING cogmap_id
        "#,
        cogmap_id,
        req.team_id,
    )
    .fetch_optional(pool)
    .await?;

    Ok(BindTeamOutcome {
        cogmap_id,
        team_id: req.team_id,
        bound: inserted.is_some(),
    })
}

// `authorable_by_profile` stood here — the named service seam for `cogmap_authorable_by_profile`,
// called by exactly two surfaces (HTTP ingest, MCP create) as a fast-fail pre-check ahead of the
// gate that actually enforces it (`DbBackend::create_resource`'s F1 `check_cogmap_authorable`).
//
// Both call sites went away when that gate learned to name the capability it withholds: a seam
// returning a bare `bool` cannot carry a refusal, so each caller had to invent its own message, and
// those messages shadowed the gate's on the most-used path into a map. Removing the last caller
// orphaned this function, and a `pub` orphan raises no warning — so it is deleted rather than left
// as a seam with nothing on either side of it. The predicate itself is untouched and still enforced;
// see `db_backend::cogmap_authorship_refusal`.

/// Unbind a cognitive map from a team (delete the `kb_team_cogmaps` row).
///
/// Auth before writes: admin-only. No-op safe — `unbound: false` when no binding
/// existed.
pub async fn unbind_team(
    pool: &PgPool,
    caller: ProfileId,
    cogmap_id: Uuid,
    team_id: Uuid,
) -> ApiResult<UnbindTeamOutcome> {
    // Auth before writes: symmetric with bind — a principal who could bind may unbind. That
    // symmetry is exactly why the shared gate excludes the gating team (see `TwoSidedAuthority`):
    // it is unbinding a gating-team-joined map, not binding one, that would be an escalation.
    crate::authz::authorize::<TwoSidedAuthority>(
        pool,
        caller,
        TwoSidedScope::cogmap(cogmap_id, team_id),
    )
    .await?;

    let result = sqlx::query!(
        "DELETE FROM kb_team_cogmaps WHERE cogmap_id = $1 AND team_id = $2",
        cogmap_id,
        team_id,
    )
    .execute(pool)
    .await?;

    Ok(UnbindTeamOutcome {
        cogmap_id,
        team_id,
        unbound: result.rows_affected() > 0,
    })
}
