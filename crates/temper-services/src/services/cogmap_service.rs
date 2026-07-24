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
use crate::error::ApiResult;
use temper_core::types::cognitive_maps::{
    BindTeamOutcome, BindTeamRequest, CogmapRow, UnbindTeamOutcome,
};
use temper_core::types::ids::{CogmapId, ProfileId};

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
          FROM cogmap_list_rows($1)
        "#,
        profile_id.uuid()
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
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

/// Producer write gate: can `profile` author a resource homed in `cogmap`?
///
/// The named service seam for the `cogmap_authorable_by_profile` SQL predicate — an explicit
/// `can_write` grant on the map (`profile_explicit_grant(...,'write','kb_cogmaps',...)`), NOT team
/// membership: cogmaps have no owner, and the Q-A flip made authorship wholly explicit (membership
/// confers read only). Surfaces (HTTP ingest, MCP create) call this as their auth-before-writes gate
/// instead of inlining the `query_scalar!` — SQL stays in the service layer, and the gate is defined
/// once rather than mirrored across surfaces. `DbBackend::create_resource` also re-enforces the same
/// predicate on the shared write path (F1), so the surface calls are fast-fail pre-checks. The
/// nullable scalar is normalized to `false` (deny) here.
pub async fn authorable_by_profile(
    pool: &PgPool,
    profile: ProfileId,
    cogmap: CogmapId,
) -> ApiResult<bool> {
    let ok = sqlx::query_scalar!(
        "SELECT cogmap_authorable_by_profile($1, $2)",
        profile.uuid(),
        cogmap.uuid()
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    Ok(ok)
}

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
