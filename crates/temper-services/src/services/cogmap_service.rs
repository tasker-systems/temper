//! Cognitive-map ↔ team binding service over the substrate.
//!
//! Service-direct, NO Backend-trait command, NO event emission — a team↔cogmap
//! binding is provisioning/infrastructure, exactly like team membership
//! (`team_service`), not knowledge-graph content (org-provisioning spec §2.6, the
//! same precedent as `context_service`). Only cogmap *genesis* got a Backend
//! command.
//!
//! Bind/unbind gate is TWO-SIDED (`can_bind`, at the TOP of each fn, BEFORE any write):
//! `is_system_admin`, OR the caller administers the MAP (`can_grant` on it) AND may manage the TEAM
//! (`can_manage` = owner/maintainer, direct membership) AND the team is NOT the gating/root team.
//! The gating-team exclusion cuts BOTH ways and the unbind direction is the load-bearing one — see
//! `can_bind`.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::{access_service, team_service};
use temper_core::types::cognitive_maps::{BindTeamOutcome, BindTeamRequest, UnbindTeamOutcome};
use temper_core::types::ids::{CogmapId, ProfileId};

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
    if !can_bind(pool, caller, cogmap_id, req.team_id).await? {
        return Err(ApiError::Forbidden);
    }

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

/// Two-sided bind/unbind gate. Allowed IFF `is_system_admin`, OR the caller can administer the MAP
/// (`can_grant` on it) AND may manage the TEAM (`can_manage` = Owner|Maintainer, direct membership)
/// AND the team is NOT the gating/root team.
///
/// **The gating-team exclusion is structural, and its load-bearing direction is UNBIND.** A
/// `kb_team_cogmaps` row joining this map to the gating team is precisely what
/// `access_service::cogmap_write_requires_admin` reads, so the binding does not merely *relate* the
/// map to a team — it *is* the switch that puts the map in the admin-write regime. Since this one
/// gate serves `unbind_team` too (a principal who could bind may unbind), dropping the exclusion
/// would let a non-admin who holds `can_grant` on the map and manages the gating team **unbind a
/// protected map**, taking it out of that regime. Binding into the gating team is a restriction the
/// caller inflicts on themselves; unbinding is an escalation. The guard exists for the second.
///
/// Its sibling `context_service::can_share` keeps the same exclusion for a different reason, and
/// `machine_authz::contain_target_team` deliberately has none. The three reasons are recorded in
/// `docs/superpowers/specs/2026-07-22-scoped-authority-policy-layer-design.md` §6.1 — they are not
/// one policy, and a future unification must not flatten them into one.
async fn can_bind(
    pool: &PgPool,
    caller: ProfileId,
    cogmap_id: Uuid,
    team_id: Uuid,
) -> ApiResult<bool> {
    if access_service::is_system_admin(pool, caller).await? {
        return Ok(true);
    }
    if access_service::is_gating_team(pool, team_id).await? {
        return Ok(false);
    }
    let team_ok = matches!(
        team_service::role_on_team(pool, team_id, caller).await?,
        Some(role) if team_service::can_manage(role)
    );
    if !team_ok {
        return Ok(false);
    }
    access_service::profile_can_grant(pool, caller, "kb_cogmaps", cogmap_id).await
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
    // Auth before writes: symmetric with bind — a principal who could bind may unbind.
    if !can_bind(pool, caller, cogmap_id, team_id).await? {
        return Err(ApiError::Forbidden);
    }

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
