//! Cognitive-map ↔ team binding service over the substrate.
//!
//! Service-direct, NO Backend-trait command, NO event emission — a team↔cogmap
//! binding is provisioning/infrastructure, exactly like team membership
//! (`team_service`), not knowledge-graph content (org-provisioning spec §2.6, the
//! same precedent as `context_service`). Only cogmap *genesis* got a Backend
//! command.
//!
//! Gating is admin-only: `is_system_admin` at the TOP of each fn, BEFORE any
//! write. (This differs from `team_service::add_member`, which gates on
//! `owner`/`maintainer` — binding a map widens its producer-intersection reach
//! across teams, so it is an operator action.)

use sqlx::PgPool;
use uuid::Uuid;

use crate::services::access_service;
use temper_core::types::cognitive_maps::{BindTeamOutcome, BindTeamRequest, UnbindTeamOutcome};
use temper_core::types::ids::ProfileId;
use temper_services::error::{ApiError, ApiResult};

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
    // Auth before writes: binding a map is a system-admin operation.
    if !access_service::is_system_admin(pool, caller).await? {
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
    // Auth before writes: unbinding a map is a system-admin operation.
    if !access_service::is_system_admin(pool, caller).await? {
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
