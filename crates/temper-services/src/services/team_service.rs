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
use temper_core::types::graph_scope::{TeamRef, TeamScopeView, TeamZone};
use temper_core::types::ids::ProfileId;
use temper_core::types::team::{
    AddMemberRequest, TeamCreateRequest, TeamDetail, TeamMemberDetail, TeamMemberRow,
    TeamMemberSource, TeamRole, TeamRow, TeamUpdateRequest,
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
        let parent_id = sqlx::query_scalar!(
            "SELECT id FROM kb_teams WHERE slug = $1 AND is_active",
            slug
        )
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
        RETURNING id, slug, name, description, created,
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
        SELECT t.id, t.slug, t.name, t.description, t.created,
               t.auto_join_role AS "auto_join_role: TeamRole"
          FROM kb_teams t
          JOIN kb_team_members tm ON tm.team_id = t.id
         WHERE tm.profile_id = $1 AND t.is_active
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
        r#"SELECT id, slug, name, description, created,
                  auto_join_role AS "auto_join_role: TeamRole"
             FROM kb_teams WHERE id = $1 AND is_active"#,
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
        description: team.description,
        created: team.created,
        auto_join_role: team.auto_join_role,
        members,
    })
}

/// Update a team's mutable metadata (name/description). Owner/maintainer only.
///
/// Partial merge: a `None` field leaves that column unchanged (SQL COALESCE).
/// A soft-deleted team is not updatable — the `is_active` filter yields
/// `NotFound`. Auth precedes the write.
pub async fn update_team(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
    req: &TeamUpdateRequest,
) -> ApiResult<TeamRow> {
    // Auth before writes: owner or maintainer.
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }

    let row = sqlx::query_as!(
        TeamRow,
        r#"
        UPDATE kb_teams
           SET name = COALESCE($2, name),
               description = COALESCE($3, description)
         WHERE id = $1 AND is_active
        RETURNING id, slug, name, description, created,
                  auto_join_role AS "auto_join_role: TeamRole"
        "#,
        team_id,
        req.name.as_deref(),
        req.description.as_deref(),
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(row)
}

/// Soft-delete a team (`is_active = false`). Owner only.
///
/// Refuses the `temper-system` root (load-bearing: every profile descends from
/// it). Idempotency: a team that is absent OR already soft-deleted yields
/// `NotFound`. Rows are preserved — recovery is a DB-level `is_active = true`.
/// Children are NOT recursively deleted (see the migration's cascade note).
pub async fn delete_team(pool: &PgPool, caller: ProfileId, team_id: Uuid) -> ApiResult<()> {
    // Auth before writes: owner only (stricter than manage — a maintainer cannot
    // dissolve the team).
    match role_on_team(pool, team_id, caller).await? {
        Some(TeamRole::Owner) => {}
        _ => return Err(ApiError::Forbidden),
    }

    // Guard the DAG root. Folded into the UPDATE's WHERE so the check and the
    // write are one atomic statement.
    let result = sqlx::query!(
        r#"UPDATE kb_teams
              SET is_active = false
            WHERE id = $1 AND is_active AND slug <> 'temper-system'"#,
        team_id,
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        // Either absent, already soft-deleted, or the protected root. The root
        // case is only reachable by its owner (auth passed), so distinguish it.
        let is_root = sqlx::query_scalar!(
            "SELECT slug = 'temper-system' FROM kb_teams WHERE id = $1",
            team_id
        )
        .fetch_optional(pool)
        .await?
        .flatten()
        .unwrap_or(false);
        return Err(if is_root {
            ApiError::Conflict("the temper-system root team cannot be deleted".to_string())
        } else {
            ApiError::NotFound
        });
    }
    Ok(())
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

    let (_target_role, source) = load_member(pool, team_id, target)
        .await?
        .ok_or(ApiError::NotFound)?;

    if matches!(source, TeamMemberSource::Idp) {
        return Err(ApiError::Conflict(
            "this membership is provisioned by SAML; change it via the identity provider"
                .to_string(),
        ));
    }

    // Last-owner guard folded into the DELETE so the count and the delete are one
    // atomic statement — two concurrent removals cannot both pass a separate count
    // check and orphan the team. The row is known to exist and be non-idp (checked
    // above), so `rows_affected() == 0` here means the guard blocked a last-owner
    // removal.
    let result = sqlx::query!(
        r#"DELETE FROM kb_team_members
            WHERE team_id = $1 AND profile_id = $2
              AND NOT (
                  role = 'owner'
                  AND (SELECT COUNT(*) FROM kb_team_members
                         WHERE team_id = $1 AND role = 'owner') = 1
              )"#,
        team_id,
        target,
    )
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "cannot remove the last owner; transfer ownership or promote another member first"
                .to_string(),
        ));
    }
    Ok(())
}

/// Change an existing member's role. Owner/maintainer only. Cannot create a
/// member (404 if absent), cannot grant `owner` (ownership is transferred, not
/// granted), refuses SAML rows, and refuses to demote the last owner.
pub async fn change_role(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
    target: Uuid,
    new_role: TeamRole,
) -> ApiResult<TeamMemberRow> {
    // Auth before writes.
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }

    if matches!(new_role, TeamRole::Owner) {
        return Err(ApiError::BadRequest(
            "cannot grant owner via role change; use ownership transfer".to_string(),
        ));
    }

    let (_current_role, source) = load_member(pool, team_id, target)
        .await?
        .ok_or(ApiError::NotFound)?;

    if matches!(source, TeamMemberSource::Idp) {
        return Err(ApiError::Conflict(
            "this membership is provisioned by SAML; change it via the identity provider"
                .to_string(),
        ));
    }

    // Demote-last-owner guard folded into the UPDATE for the same atomicity reason as
    // `remove_member`. `new_role` is already known to be non-owner (checked above), so
    // any update to a sole owner is a demotion and the guard blocks it — a `None`
    // result means the last-owner guard fired.
    let row = sqlx::query_as!(
        TeamMemberRow,
        r#"UPDATE kb_team_members SET role = $3
            WHERE team_id = $1 AND profile_id = $2
              AND NOT (
                  role = 'owner'
                  AND (SELECT COUNT(*) FROM kb_team_members
                         WHERE team_id = $1 AND role = 'owner') = 1
              )
        RETURNING team_id, profile_id, role AS "role: TeamRole", created"#,
        team_id,
        target,
        new_role as TeamRole,
    )
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| {
        ApiError::Conflict(
            "cannot demote the last owner; transfer ownership or promote another member first"
                .to_string(),
        )
    })
}

/// R1 team-graph-scope read: the scope team, its reachable ancestors, and the
/// child-team zones the profile may enter. Deny-as-absence (404) when the profile
/// cannot view the team (not a member of the team or any of its descendants).
pub async fn graph_scope(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    team_id: uuid::Uuid,
) -> ApiResult<TeamScopeView> {
    // Access gate: the profile must be a member of the team or a descendant (upward read).
    let viewable: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM team_descendants($1) d
            JOIN kb_team_members tm ON tm.team_id = d.team_id AND tm.profile_id = $2
        )",
    )
    .bind(team_id)
    .bind(*profile_id)
    .fetch_one(pool)
    .await?;
    if !viewable {
        return Err(ApiError::NotFound);
    }

    // The scope team itself. (The viewable gate above already implies a member row —
    // hence the team — exists; the ok_or is defensive belt-and-suspenders.)
    let team: TeamRef = sqlx::query_as::<_, (uuid::Uuid, String, String)>(
        "SELECT id, slug, name FROM kb_teams WHERE id = $1",
    )
    .bind(team_id)
    .fetch_optional(pool)
    .await?
    .map(|(id, slug, name)| TeamRef { id, slug, name })
    .ok_or(ApiError::NotFound)?;

    // Reachable ancestors (up-set, excluding self).
    let ancestors: Vec<TeamRef> = sqlx::query_as::<_, (uuid::Uuid, String, String)>(
        "SELECT t.id, t.slug, t.name
           FROM team_ancestors($1) a
           JOIN kb_teams t ON t.id = a.team_id
          WHERE a.team_id <> $1
          ORDER BY t.name",
    )
    .bind(team_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, slug, name)| TeamRef { id, slug, name })
    .collect();

    // Enterable child zones + size hint (count of resources in the child's scope).
    let zones: Vec<TeamZone> = sqlx::query_as::<_, (uuid::Uuid, String, String, i32)>(
        "SELECT t.id, t.slug, t.name,
                (SELECT count(*) FROM resources_in_team_scope($2, t.id))::int AS resource_count
           FROM team_child_zones($2, $1) z
           JOIN kb_teams t ON t.id = z.team_id
          ORDER BY t.name",
    )
    .bind(team_id)
    .bind(*profile_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, slug, name, resource_count)| TeamZone {
        id,
        slug,
        name,
        resource_count,
    })
    .collect();

    Ok(TeamScopeView {
        team,
        ancestors,
        zones,
    })
}

#[cfg(all(test, feature = "test-db"))]
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

    /// Insert a context owned by a team, return its id.
    async fn mk_team_context(pool: &PgPool, team: Uuid, slug: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_teams', $1, $2, $2) RETURNING id",
        )
        .bind(team)
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Insert a resource homed in `ctx`, owned+originated by `owner` (so it is
    /// visible to nobody else by ownership). Return its id.
    async fn mk_homed_resource(pool: &PgPool, ctx: Uuid, owner: Uuid) -> Uuid {
        let rid: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('r', 'r') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_resource_homes \
                (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_contexts', $2, $3, $3)",
        )
        .bind(rid)
        .bind(ctx)
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
        rid
    }

    async fn is_visible(pool: &PgPool, profile: Uuid, resource: Uuid) -> bool {
        sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
            profile,
            resource,
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .unwrap_or(false)
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn owner_changes_member_role(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let member = mk_profile(&pool, "member").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, member, "member", "native").await;

        let row = change_role(
            &pool,
            ProfileId::from(owner),
            team,
            member,
            TeamRole::Maintainer,
        )
        .await
        .unwrap();
        assert!(matches!(row.role, TeamRole::Maintainer));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_grant_owner_via_role_change(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let member = mk_profile(&pool, "member").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, member, "member", "native").await;

        let denied =
            change_role(&pool, ProfileId::from(owner), team, member, TeamRole::Owner).await;
        assert!(matches!(denied, Err(ApiError::BadRequest(_))));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_demote_last_owner(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        let denied = change_role(
            &pool,
            ProfileId::from(owner),
            team,
            owner,
            TeamRole::Maintainer,
        )
        .await;
        assert!(matches!(denied, Err(ApiError::Conflict(_))));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn change_role_on_nonmember_is_not_found(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let ghost = mk_profile(&pool, "ghost").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        let denied =
            change_role(&pool, ProfileId::from(owner), team, ghost, TeamRole::Member).await;
        assert!(matches!(denied, Err(ApiError::NotFound)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_change_role_of_idp_row(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let idp = mk_profile(&pool, "idp").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, idp, "member", "idp").await;

        let denied = change_role(
            &pool,
            ProfileId::from(owner),
            team,
            idp,
            TeamRole::Maintainer,
        )
        .await;
        assert!(matches!(denied, Err(ApiError::Conflict(_))));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn maintainer_can_remove_a_member(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let maintainer = mk_profile(&pool, "maintainer").await;
        let member = mk_profile(&pool, "member").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, maintainer, "maintainer", "native").await;
        add(&pool, team, member, "member", "native").await;

        // A maintainer (not just an owner) may manage membership.
        remove_member(&pool, ProfileId::from(maintainer), team, member)
            .await
            .unwrap();
        let detail = team_detail(&pool, ProfileId::from(owner), team)
            .await
            .unwrap();
        assert_eq!(detail.members.len(), 2);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn team_detail_visible_to_system_admin_non_member(pool: PgPool) {
        let admin = mk_profile(&pool, "admin").await;
        let owner = mk_profile(&pool, "owner").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        // Make `admin` a system admin: OWNER of the `temper-system` gating team (born of
        // migration 20260625000001) with `gating_team_slug` pointed at it — the same
        // admin-minting idiom as `context_service`'s test seed. `admin` is NOT a member
        // of `acme`, so this exercises the `is_system_admin` branch of the read gate.
        let sys: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
            .fetch_one(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner') \
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = 'owner'",
        )
        .bind(sys)
        .bind(admin)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1",
        )
        .execute(&pool)
        .await
        .unwrap();

        let detail = team_detail(&pool, ProfileId::from(admin), team)
            .await
            .unwrap();
        assert_eq!(detail.slug, "acme");
        assert_eq!(detail.members.len(), 1);
    }

    // --- Team metadata + soft-delete (scope task #5) ---

    #[sqlx::test(migrations = "../../migrations")]
    async fn owner_updates_name_and_description(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        let req = TeamUpdateRequest {
            name: Some("Acme Inc".to_string()),
            description: Some("the roadrunner people".to_string()),
        };
        let row = update_team(&pool, ProfileId::from(owner), team, &req)
            .await
            .unwrap();
        assert_eq!(row.name, "Acme Inc");
        assert_eq!(row.description.as_deref(), Some("the roadrunner people"));

        // Partial merge: updating only the name leaves the description intact.
        let req2 = TeamUpdateRequest {
            name: Some("Acme LLC".to_string()),
            description: None,
        };
        let row2 = update_team(&pool, ProfileId::from(owner), team, &req2)
            .await
            .unwrap();
        assert_eq!(row2.name, "Acme LLC");
        assert_eq!(row2.description.as_deref(), Some("the roadrunner people"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn non_manager_cannot_update(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let member = mk_profile(&pool, "member").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, member, "member", "native").await;

        let req = TeamUpdateRequest {
            name: Some("hijack".to_string()),
            description: None,
        };
        let denied = update_team(&pool, ProfileId::from(member), team, &req).await;
        assert!(matches!(denied, Err(ApiError::Forbidden)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn update_soft_deleted_team_is_not_found(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        delete_team(&pool, ProfileId::from(owner), team)
            .await
            .unwrap();
        let req = TeamUpdateRequest {
            name: Some("ghost".to_string()),
            description: None,
        };
        let denied = update_team(&pool, ProfileId::from(owner), team, &req).await;
        assert!(matches!(denied, Err(ApiError::NotFound)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn owner_soft_deletes_team_and_it_disappears(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        // `acme` is listed while active (the caller also has an auto-provisioned
        // personal team, so assert on membership of `acme` specifically, not a count).
        let before = list_teams(&pool, ProfileId::from(owner)).await.unwrap();
        assert!(before.iter().any(|t| t.id == team));

        delete_team(&pool, ProfileId::from(owner), team)
            .await
            .unwrap();

        // Gone from the caller's listing and no longer showable.
        let after = list_teams(&pool, ProfileId::from(owner)).await.unwrap();
        assert!(!after.iter().any(|t| t.id == team));
        let shown = team_detail(&pool, ProfileId::from(owner), team).await;
        assert!(matches!(shown, Err(ApiError::NotFound)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn maintainer_cannot_delete_team(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let maintainer = mk_profile(&pool, "maintainer").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, maintainer, "maintainer", "native").await;

        // A maintainer may manage membership but NOT dissolve the team.
        let denied = delete_team(&pool, ProfileId::from(maintainer), team).await;
        assert!(matches!(denied, Err(ApiError::Forbidden)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_delete_temper_system_root(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let sys: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
            .fetch_one(&pool)
            .await
            .unwrap();
        // Profiles auto-join temper-system via the sync_system_membership trigger, so
        // upsert the owner role rather than insert a fresh row.
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner') \
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = 'owner'",
        )
        .bind(sys)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();

        let denied = delete_team(&pool, ProfileId::from(owner), sys).await;
        assert!(matches!(denied, Err(ApiError::Conflict(_))));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn soft_deleted_team_stops_conferring_read_reach(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let member = mk_profile(&pool, "member").await;
        let stranger = mk_profile(&pool, "stranger").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, member, "member", "native").await;

        // A resource homed in a team-owned context, owned by an unrelated profile —
        // so `member`'s ONLY path to it is team membership, not ownership.
        let ctx = mk_team_context(&pool, team, "acme-ctx").await;
        let resource = mk_homed_resource(&pool, ctx, stranger).await;

        // While the team is active, the member sees it via the team-owned-context branch.
        assert!(is_visible(&pool, member, resource).await);

        // Soft-delete the team → the read-reach evaporates for the member...
        delete_team(&pool, ProfileId::from(owner), team)
            .await
            .unwrap();
        assert!(!is_visible(&pool, member, resource).await);

        // ...while the resource's own owner still sees it (unaffected by the team).
        assert!(is_visible(&pool, stranger, resource).await);
    }
}
