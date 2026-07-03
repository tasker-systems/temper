//! Team lifecycle service over the substrate.
//!
//! Service-direct, NO Backend-trait command, NO event emission — teams are
//! provisioning/infrastructure, not knowledge-graph content (org-provisioning
//! spec §2.6, the same precedent as `context_service`). The only pre-existing
//! team write (approval, `access_service::review_request`) is likewise
//! service-local.
//!
//! Role-gating is pure authz over `kb_team_members.role` + the `kb_teams_parents`
//! DAG (spec §3 decision #1): anyone may create a **root** (parentless) team and
//! becomes its `owner`; creating a **child** requires `owner`/`maintainer` on the
//! parent; setting `auto_join_role` requires `is_system_admin`. Auth checks
//! precede every write.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::access_service;
use temper_core::types::ids::ProfileId;
use temper_core::types::team::{
    AddMemberRequest, TeamCreateRequest, TeamDetail, TeamMemberDetail, TeamMemberRow,
    TeamMemberSource, TeamRole, TeamRow,
};

/// Map a sqlx error to `Conflict` when it is a unique-constraint violation
/// (the globally-UNIQUE `kb_teams.slug`), else pass it through.
fn map_unique_violation(err: sqlx::Error, message: &str) -> ApiError {
    if let sqlx::Error::Database(db) = &err {
        if db.is_unique_violation() {
            return ApiError::Conflict(message.to_string());
        }
    }
    ApiError::from(err)
}

/// Strip an optional `+` sigil from a team ref, yielding the bare slug.
fn team_slug(parent_ref: &str) -> &str {
    parent_ref.strip_prefix('+').unwrap_or(parent_ref)
}

/// Fetch the caller's role on a team, if any.
///
/// `pub(crate)` so sibling services (e.g. `context_service`'s team-owned-context
/// gate) reuse the one role check rather than duplicating the authz.
pub(crate) async fn role_on_team(
    pool: &PgPool,
    team_id: Uuid,
    profile_id: ProfileId,
) -> ApiResult<Option<TeamRole>> {
    let role = sqlx::query_scalar!(
        r#"SELECT role AS "role: TeamRole"
             FROM kb_team_members
            WHERE team_id = $1 AND profile_id = $2"#,
        team_id,
        *profile_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(role)
}

/// True if the role is `owner` or `maintainer` (may manage the team).
///
/// `pub(crate)` so sibling services reuse the one definition (see `role_on_team`).
pub(crate) fn can_manage(role: TeamRole) -> bool {
    matches!(role, TeamRole::Owner | TeamRole::Maintainer)
}

/// Create a team. The caller becomes its `owner`.
///
/// Auth before writes:
/// - **child** (`parent` set): caller must be `owner`/`maintainer` on the parent.
/// - **root** (`parent` None): any authenticated profile may create.
/// - `auto_join_role` set: caller must be `is_system_admin`.
///
/// All inserts run in one transaction: `kb_teams`, the optional
/// `kb_teams_parents` link, and the creator's `owner` membership. After commit,
/// if `auto_join_role` was set, `backfill_auto_join_team` enrolls existing
/// eligible profiles (the creator-owner row is preserved — backfill is
/// `ON CONFLICT DO NOTHING`).
pub async fn create_team(
    pool: &PgPool,
    creator: ProfileId,
    req: &TeamCreateRequest,
) -> ApiResult<TeamRow> {
    // --- Auth before writes ---

    // Child team: resolve the parent and require owner/maintainer on it.
    let parent_id = if let Some(parent_ref) = &req.parent {
        let slug = team_slug(parent_ref);
        let parent_id = sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = $1", slug)
            .fetch_optional(pool)
            .await?
            .ok_or(ApiError::NotFound)?;
        match role_on_team(pool, parent_id, creator).await? {
            Some(role) if can_manage(role) => {}
            _ => return Err(ApiError::Forbidden),
        }
        Some(parent_id)
    } else {
        None
    };

    // auto_join_role defines an everyone-pool — admin-gated.
    if req.auto_join_role.is_some() && !access_service::is_system_admin(pool, creator).await? {
        return Err(ApiError::Forbidden);
    }

    // --- Writes (one transaction) ---
    let team_id = Uuid::now_v7();
    let name = req.name.clone().unwrap_or_else(|| req.slug.clone());

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    let row = sqlx::query_as!(
        TeamRow,
        r#"
        INSERT INTO kb_teams (id, slug, name, auto_join_role)
        VALUES ($1, $2, $3, $4)
        RETURNING id, slug, name, created,
                  auto_join_role AS "auto_join_role: TeamRole"
        "#,
        team_id,
        req.slug,
        name,
        req.auto_join_role as Option<TeamRole>,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| map_unique_violation(e, "team slug already exists"))?;

    if let Some(parent_id) = parent_id {
        sqlx::query!(
            "INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)",
            team_id,
            parent_id,
        )
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query!(
        r#"INSERT INTO kb_team_members (team_id, profile_id, role)
           VALUES ($1, $2, 'owner')"#,
        team_id,
        *creator,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    // After commit: enroll existing eligible profiles into the new auto-join pool.
    if req.auto_join_role.is_some() {
        sqlx::query!("SELECT backfill_auto_join_team($1)", team_id)
            .execute(pool)
            .await?;
    }

    Ok(row)
}

/// Add (or update) a member on a team. The caller must be `owner`/`maintainer`.
pub async fn add_member(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
    req: &AddMemberRequest,
) -> ApiResult<TeamMemberRow> {
    // Auth before writes.
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }

    let row = sqlx::query_as!(
        TeamMemberRow,
        r#"
        INSERT INTO kb_team_members (team_id, profile_id, role)
        VALUES ($1, $2, $3)
        ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role
        RETURNING team_id, profile_id, role AS "role: TeamRole", created
        "#,
        team_id,
        req.profile_id,
        req.role as TeamRole,
    )
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// List the teams the caller is a member of.
pub async fn list_teams(pool: &PgPool, caller: ProfileId) -> ApiResult<Vec<TeamRow>> {
    let rows = sqlx::query_as!(
        TeamRow,
        r#"
        SELECT t.id, t.slug, t.name, t.created,
               t.auto_join_role AS "auto_join_role: TeamRole"
          FROM kb_teams t
          JOIN kb_team_members tm ON tm.team_id = t.id
         WHERE tm.profile_id = $1
         ORDER BY t.name
        "#,
        *caller,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Full team detail (row + member roster with handles + provenance).
///
/// Visible to any member of the team, or to a system admin. Non-visible teams
/// return `NotFound` (not `Forbidden`) to avoid leaking team existence to
/// non-members — team slugs are globally unique and used in share flows.
pub async fn team_detail(pool: &PgPool, caller: ProfileId, team_id: Uuid) -> ApiResult<TeamDetail> {
    // Auth (read gate): member (any role) or system admin.
    let is_member = role_on_team(pool, team_id, caller).await?.is_some();
    if !is_member && !access_service::is_system_admin(pool, caller).await? {
        return Err(ApiError::NotFound);
    }

    let team = sqlx::query_as!(
        TeamRow,
        r#"SELECT id, slug, name, created,
                  auto_join_role AS "auto_join_role: TeamRole"
             FROM kb_teams WHERE id = $1"#,
        team_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    let members = sqlx::query_as!(
        TeamMemberDetail,
        r#"SELECT tm.profile_id,
                  p.handle,
                  tm.role AS "role: TeamRole",
                  tm.source AS "source: TeamMemberSource"
             FROM kb_team_members tm
             JOIN kb_profiles p ON p.id = tm.profile_id
            WHERE tm.team_id = $1
            ORDER BY tm.role, p.handle"#,
        team_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(TeamDetail {
        id: team.id,
        slug: team.slug,
        name: team.name,
        created: team.created,
        auto_join_role: team.auto_join_role,
        members,
    })
}

/// Count the `owner`-role members of a team.
async fn count_owners(pool: &PgPool, team_id: Uuid) -> ApiResult<i64> {
    let n = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM kb_team_members WHERE team_id = $1 AND role = 'owner'",
        team_id,
    )
    .fetch_one(pool)
    .await?;
    Ok(n.unwrap_or(0))
}

/// Load a member's role + provenance, if the row exists.
async fn load_member(
    pool: &PgPool,
    team_id: Uuid,
    profile: Uuid,
) -> ApiResult<Option<(TeamRole, TeamMemberSource)>> {
    let row = sqlx::query!(
        r#"SELECT role AS "role: TeamRole", source AS "source: TeamMemberSource"
             FROM kb_team_members WHERE team_id = $1 AND profile_id = $2"#,
        team_id,
        profile,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.role, r.source)))
}

/// Remove a member from a team. Owner/maintainer may remove others; any member
/// may remove themselves (self-leave). Refuses SAML-provisioned rows and refuses
/// to remove the last owner.
pub async fn remove_member(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
    target: Uuid,
) -> ApiResult<()> {
    // Auth before writes: manager, or self-leave.
    let is_self = *caller == target;
    if !is_self {
        match role_on_team(pool, team_id, caller).await? {
            Some(role) if can_manage(role) => {}
            _ => return Err(ApiError::Forbidden),
        }
    }

    let (target_role, source) = load_member(pool, team_id, target)
        .await?
        .ok_or(ApiError::NotFound)?;

    if matches!(source, TeamMemberSource::Idp) {
        return Err(ApiError::Conflict(
            "this membership is provisioned by SAML; change it via the identity provider"
                .to_string(),
        ));
    }
    if matches!(target_role, TeamRole::Owner) && count_owners(pool, team_id).await? == 1 {
        return Err(ApiError::Conflict(
            "cannot remove the last owner; transfer ownership or promote another member first"
                .to_string(),
        ));
    }

    sqlx::query!(
        "DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
        team_id,
        target,
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use sqlx::PgPool;
    use temper_core::types::team::{TeamMemberSource, TeamRole};

    /// Insert a profile with the given handle, return its id.
    async fn mk_profile(pool: &PgPool, handle: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Insert a root team with the given slug, return its id.
    async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_teams (id, slug, name) VALUES (gen_random_uuid(), $1, $1) RETURNING id",
        )
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn add(pool: &PgPool, team: Uuid, profile: Uuid, role: &str, source: &str) {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
             VALUES ($1, $2, $3::team_role, $4::team_member_source)",
        )
        .bind(team)
        .bind(profile)
        .bind(role)
        .bind(source)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn team_detail_lists_members_for_a_member(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let member = mk_profile(&pool, "member").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, member, "member", "native").await;

        let detail = team_detail(&pool, ProfileId::from(owner), team)
            .await
            .unwrap();
        assert_eq!(detail.slug, "acme");
        assert_eq!(detail.members.len(), 2);
        assert!(detail.members.iter().any(|m| m.handle == "member"
            && matches!(m.role, TeamRole::Member)
            && matches!(m.source, TeamMemberSource::Native)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn team_detail_hides_from_non_member(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let outsider = mk_profile(&pool, "outsider").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        let denied = team_detail(&pool, ProfileId::from(outsider), team).await;
        assert!(matches!(denied, Err(ApiError::NotFound)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn owner_removes_member(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let member = mk_profile(&pool, "member").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, member, "member", "native").await;

        remove_member(&pool, ProfileId::from(owner), team, member)
            .await
            .unwrap();
        let detail = team_detail(&pool, ProfileId::from(owner), team)
            .await
            .unwrap();
        assert_eq!(detail.members.len(), 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn member_can_self_leave_but_not_remove_others(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let a = mk_profile(&pool, "a").await;
        let b = mk_profile(&pool, "b").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, a, "member", "native").await;
        add(&pool, team, b, "member", "native").await;

        // a removing b → Forbidden.
        let denied = remove_member(&pool, ProfileId::from(a), team, b).await;
        assert!(matches!(denied, Err(ApiError::Forbidden)));
        // a removing a (self-leave) → ok.
        remove_member(&pool, ProfileId::from(a), team, a)
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_remove_last_owner(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        let denied = remove_member(&pool, ProfileId::from(owner), team, owner).await;
        assert!(matches!(denied, Err(ApiError::Conflict(_))));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_remove_idp_row(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let idp = mk_profile(&pool, "idp").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, idp, "member", "idp").await;

        let denied = remove_member(&pool, ProfileId::from(owner), team, idp).await;
        assert!(matches!(denied, Err(ApiError::Conflict(_))));
    }
}
