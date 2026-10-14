//! Team lifecycle service over the substrate.
//!
//! Service-direct, NO Backend-trait command — teams are provisioning/infrastructure, not
//! knowledge-graph content (org-provisioning spec §2.6, the same precedent as `context_service`).
//! No cognition events. Membership acts are the exception that proves the rule: `add_member`,
//! `change_role` and `remove_member` are authority acts, decided by the rank rule in
//! `authz::team_membership` and recorded on the ADMIN ledger (`team_membership_changed`, NULL-
//! anchored), atomically with the row (team-role rank spec C8).
//!
//! Role-gating is pure authz over `kb_team_members.role` + the `kb_teams_parents`
//! DAG (spec §3 decision #1): anyone may create a **root** (parentless) team and
//! becomes its `owner`; creating a **child** requires `owner`/`maintainer` on the
//! parent; setting `auto_join_role` requires `is_system_admin`. Auth checks
//! precede every write.

use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::{Caller, HumanPrincipal};
use crate::authz::team_membership::{self, MembershipAct, MembershipAuthority};
use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::ProfileId;
use temper_core::types::reassign::{RemoveMemberOutcome, ResidualContext, ResidualOwnedReach};
use temper_core::types::team::{
    AddMemberRequest, TeamCreateRequest, TeamDetail, TeamMemberDetail, TeamMemberRow,
    TeamMemberSource, TeamRole, TeamRow, TeamUpdateRequest,
};

/// The slug prefix of every personal team (`personal-<handle>`), which `sync_personal_team`
/// alone may create.
pub const PERSONAL_TEAM_SLUG_PREFIX: &str = "personal-";

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
///
/// **Deliberately `is_active`-blind.** It answers *"is there a membership row?"*, not *"is this
/// team effective?"* — `20260703000001_team_metadata_soft_delete.sql`'s invariant
/// (*"membership in a soft-deleted team confers nothing anywhere"*) belongs on the **team**, which
/// is where the SQL chokepoints put it too (`profile_effective_teams` filters `kb_teams`, not
/// `kb_team_members`). A caller that must not act on a dead team states it on its own team lookup:
/// `… WHERE slug = $1 AND is_active` in `create_team`'s parent resolution and in
/// `context_service::{resolve_context_ref, resolve_create_owner}`, the EXISTS in
/// `reassign_service::reassign_team_resources`. Filtering here instead would restate it where it
/// cannot be violated, imply those gates were optional, and silently change `delete_team` /
/// `update_team`, which need the blind answer to tell an already-deleted team from a non-member.
/// So the rule for a new call site is: gate the team there, not here.
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

/// [`role_on_team`] on a connection — for a decision made inside a write transaction, under its
/// locks (team-role rank plan P1). The same query, equally `is_active`-blind; the pool spelling is
/// left alone.
pub(crate) async fn role_on_team_on(
    conn: &mut sqlx::PgConnection,
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
    .fetch_optional(&mut *conn)
    .await?;
    Ok(role)
}

/// True if the role is `owner` or `maintainer` (may manage the team).
///
/// `pub(crate)` so sibling services reuse the one definition (see `role_on_team`).
pub(crate) fn can_manage(role: TeamRole) -> bool {
    matches!(role, TeamRole::Owner | TeamRole::Maintainer)
}

/// "This caller holds a manage-capable role on this team" — the bar for conferring connection
/// reach on a target team (`machine_authz::contain_target_team`, for
/// `connection_service::grant_reach`).
///
/// It is NOT what `add_member` requires: that is the rank rule in `authz::team_membership`, which
/// also refuses a non-admin on the gating team (team-role rank spec R11). `machine_authz::contain_reach`
/// writes membership rows, so its team loop asks `MembershipAuthority` instead (plan P15); a reach
/// grant writes no membership, which is why this bar keeps no gating-team exclusion (see
/// `contain_target_team`).
///
/// A nonexistent team also lands here: `role_on_team` returns `None` for a `team_id` no row
/// carries, so a bogus UUID is `Forbidden` and never reaches the write. That matters because
/// `kb_access_grants.principal_id` has **no foreign key** — without this, a made-up UUID would
/// silently persist a dangling grant row. No separate existence query is needed on this path.
pub(crate) async fn require_manage_on_team(
    pool: &PgPool,
    team_id: Uuid,
    caller: ProfileId,
) -> ApiResult<()> {
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => Ok(()),
        _ => Err(ApiError::Forbidden),
    }
}

/// Assert a team row exists, for callers whose authority skips [`require_manage_on_team`] (a
/// system admin) and who would otherwise write an unvalidated `principal_id`. `kb_access_grants`
/// carries no FK on `principal_id`, so this is the only thing standing between an admin's typo
/// and a dangling grant nobody can see or revoke by team name.
pub(crate) async fn require_team_exists(pool: &PgPool, team_id: Uuid) -> ApiResult<()> {
    let exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_teams WHERE id = $1) AS "e!: bool""#,
        team_id,
    )
    .fetch_one(pool)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(ApiError::NotFound(
            "team not found or not readable".to_string(),
        ))
    }
}

/// Create a team. The caller becomes its `owner`.
///
/// The [`PERSONAL_TEAM_SLUG_PREFIX`] is refused for every caller, admins included: only the
/// `sync_personal_team` trigger creates a personal team. The trigger no longer joins a profile to
/// a team it did not create (a held slug sends it to `personal-<handle>-N`), so the refusal is the
/// first of two guards against squatting, and keeps `personal-<handle>` meaning that profile's.
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
    authed: &HumanPrincipal,
    req: &TeamCreateRequest,
) -> ApiResult<TeamRow> {
    let creator = ProfileId::from(authed.profile().id);
    if req
        .slug
        .to_ascii_lowercase()
        .starts_with(PERSONAL_TEAM_SLUG_PREFIX)
    {
        return Err(ApiError::BadRequest(format!(
            "the `{PERSONAL_TEAM_SLUG_PREFIX}` slug prefix is reserved for personal teams"
        )));
    }

    // A team's creator becomes its `owner`, and a machine principal never governs (ruled
    // 2026-10-09; the database refuses the row — 20261019100000_machines_never_govern.sql). The
    // `&HumanPrincipal` parameter is what keeps a machine out: the classification that minted it
    // already read `is_machine_profile`.

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
        .ok_or_else(|| ApiError::NotFound("parent team not found or not readable".to_string()))?;
        match role_on_team(pool, parent_id, creator).await? {
            Some(role) if can_manage(role) => {}
            _ => return Err(ApiError::Forbidden),
        }
        Some(parent_id)
    } else {
        None
    };

    // auto_join_role defines an everyone-pool — admin-gated.
    if req.auto_join_role.is_some()
        && crate::authz::Principal::Human(authed)
            .system_admin(pool)
            .await?
            .is_none()
    {
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

/// Add a member to a team at `req.role` — the rank rule (team-role rank spec C1, R11): a manager
/// can grant a role up to their own (an owner can grant `owner`, a maintainer up to `maintainer`);
/// only a system admin can add to the gating team, and a system admin can grant any role. A
/// machine target is capped at `member` for everyone (C5). The add and its
/// `team_membership_changed` record are one write.
///
/// **Insert-only.** A profile already on the team is a `Conflict` naming `change_role`, never a role
/// update (#1070): changing a role is `change_role`'s act, under its own checks.
pub async fn add_member(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    req: &AddMemberRequest,
) -> ApiResult<TeamMemberRow> {
    let target = ProfileId::from(req.profile_id);
    let (mut tx, change) = authorize_member_act(
        pool,
        caller,
        team_id,
        target,
        MembershipAct::Add { role: req.role },
    )
    .await?;
    team_membership::membership_write(&mut tx, None, change).await?;
    let row = member_row(&mut tx, team_id, target).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;
    Ok(row)
}

/// Run one member act up to its sealed authorization, in plan P1's order, and hand back the open
/// transaction that holds its locks:
///
/// 1. **Pre-check on the pool.** Resolve the caller's authority and apply the half of the rank rule
///    that needs no target state. A caller with no authority — or a member acting on someone else —
///    is refused here, before any lock, and learns nothing about the team or the target (S6).
/// 2. **Lock** the team row, then the target's row ([`team_membership::locked`]).
/// 3. **Re-resolve** the authority on the transaction, under the locks, so the decision reads
///    current state: a caller demoted a moment ago acts on their new role, and two co-owners
///    leaving at once are serialized (C6).
/// 4. **Authorize** the act against the target as locked.
///
/// The caller writes with [`team_membership::membership_write`] and commits.
async fn authorize_member_act(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    target: ProfileId,
    act: MembershipAct,
) -> ApiResult<(
    sqlx::Transaction<'static, sqlx::Postgres>,
    team_membership::AuthorizedMembershipChange,
)> {
    {
        let mut conn = pool.acquire().await?;
        MembershipAuthority::resolve(&mut conn, caller, team_id)
            .await?
            .precheck(caller, act, target)?;
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;
    let locked = team_membership::locked(&mut tx, team_id, target).await?;
    let resolved = MembershipAuthority::resolve(&mut tx, caller, team_id).await?;
    let change = team_membership::authorize(resolved, caller, team_id, act, &locked)?;
    Ok((tx, change))
}

/// The member's row as it stands on `conn`, for an act's response.
async fn member_row(
    conn: &mut sqlx::PgConnection,
    team_id: Uuid,
    profile: ProfileId,
) -> ApiResult<TeamMemberRow> {
    Ok(sqlx::query_as!(
        TeamMemberRow,
        r#"SELECT team_id, profile_id, role AS "role: TeamRole", created
             FROM kb_team_members
            WHERE team_id = $1 AND profile_id = $2"#,
        team_id,
        *profile,
    )
    .fetch_one(&mut *conn)
    .await?)
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
pub async fn team_detail(pool: &PgPool, caller: &Caller, team_id: Uuid) -> ApiResult<TeamDetail> {
    // Auth (read gate): member (any role) or system admin. The `NotFound` above is rendered by
    // `TeamReadAuthority::denial`, so the information-hiding decision lives with the policy rather
    // than being re-made at each call site.
    crate::authz::authorize::<crate::authz::TeamReadAuthority>(
        pool,
        crate::authz::Principal::from(caller),
        team_id,
    )
    .await?;

    let team = sqlx::query_as!(
        TeamRow,
        r#"SELECT id, slug, name, description, created,
                  auto_join_role AS "auto_join_role: TeamRole"
             FROM kb_teams WHERE id = $1 AND is_active"#,
        team_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("team not found or not readable".to_string()))?;

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
    caller: &HumanPrincipal,
    team_id: Uuid,
    req: &TeamUpdateRequest,
) -> ApiResult<TeamRow> {
    // Auth before writes: owner or maintainer.
    match role_on_team(pool, team_id, caller.profile_id()).await? {
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
    .ok_or_else(|| ApiError::NotFound("team not found or not readable".to_string()))?;

    Ok(row)
}

/// Soft-delete a team (`is_active = false`). Owner only.
///
/// Refuses the `temper-system` root (load-bearing: every profile descends from
/// it). Idempotency: a team that is absent OR already soft-deleted yields
/// `NotFound`. Rows are preserved — recovery is a DB-level `is_active = true`.
/// Children are NOT recursively deleted (see the migration's cascade note).
pub async fn delete_team(pool: &PgPool, caller: &HumanPrincipal, team_id: Uuid) -> ApiResult<()> {
    // Auth before writes: owner only (stricter than manage — a maintainer cannot
    // dissolve the team).
    match role_on_team(pool, team_id, caller.profile_id()).await? {
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
            ApiError::NotFound("team not found or not readable".to_string())
        });
    }
    Ok(())
}

/// Remove a member from a team. A manager removes members up to their own rank who are not owners;
/// anyone may leave (team-role rank spec C2, C3, R6). Only a system admin removes another owner, or
/// anyone from the gating team other than themselves (R11). Refuses an identity-provider row (C7),
/// a profile's own personal team, and the team's last owner (C6). The removal and its
/// `team_membership_changed` record are one write.
pub async fn remove_member(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    target: Uuid,
) -> ApiResult<RemoveMemberOutcome> {
    let (mut tx, change) = authorize_member_act(
        pool,
        caller,
        team_id,
        ProfileId::from(target),
        MembershipAct::Remove,
    )
    .await?;

    // A profile's personal team is its own for as long as the profile exists. For a person the
    // last-owner guard already holds them there; a machine holds its personal team as `member`
    // (20261019100000_machines_never_govern.sql), so without this anyone who manages that team could
    // remove it, cutting its reach to the root. (The machine itself never reaches here: the caller
    // is a person.) Asked only AFTER the act is authorized, so a caller who could not act learns
    // nothing about whose personal team this is (S6).
    let personal: bool = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM kb_teams WHERE id = $1 AND personal_of = $2) AS "p!: bool""#,
        team_id,
        target,
    )
    .fetch_one(&mut *tx)
    .await?;
    if personal {
        return Err(ApiError::Conflict(
            "a profile cannot be removed from its own personal team".to_string(),
        ));
    }

    team_membership::membership_write(&mut tx, None, change).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    // Removal succeeded. Surface (read-only) the reach the removed member still
    // OWNS in this team's contexts (live resources only; a soft-deleted or erased one
    // is nothing to hand off), so the caller can hand it off deliberately via
    // `reassign_team_resources`. The scope query is membership-independent, so it is
    // correct post-delete; we reuse the exact definition the handoff moves, so the
    // warning and the handoff can never disagree.
    let owned = crate::services::reassign_service::team_scoped_owned(pool, team_id, target).await?;
    // Group by context IDENTITY (context_id), not by row adjacency: a slug is unique
    // only per-owner, so two distinct contexts can share a slug and interleave in the
    // result — an adjacency fold would split one into duplicate entries. A map keyed
    // on context_id is order-independent and preserves first-seen order for stable output.
    let mut contexts: Vec<ResidualContext> = Vec::new();
    let mut index: std::collections::HashMap<Uuid, usize> = std::collections::HashMap::new();
    for row in &owned {
        match index.get(&row.context_id) {
            Some(&i) => contexts[i].count += 1,
            None => {
                index.insert(row.context_id, contexts.len());
                contexts.push(ResidualContext {
                    context_ref: row.context_ref.clone(),
                    count: 1,
                });
            }
        }
    }
    Ok(RemoveMemberOutcome {
        residual_owned: ResidualOwnedReach {
            count: owned.len() as u32,
            contexts,
        },
    })
}

/// Change an existing member's role — the rank rule (team-role rank spec C1–C3a, R6, R11):
/// - a manager can set a member's role up to their own, on members up to their own rank who are
///   not owners (a maintainer acts on a maintainer; only a system admin changes an owner other
///   than yourself);
/// - an owner or maintainer can lower their own role (to hand a team over, grant `owner` to another
///   member, then lower yourself), but no one can raise their own;
/// - on the gating team, only a system admin changes roles;
/// - a system admin can make any change.
///
/// `404` if the profile is not a member, `409` on an identity-provider row (C7) or when the change
/// would leave the team without an owner (C6). A change to the member's current role is a `200` and
/// records nothing (C8). The change and its `team_membership_changed` record are one write.
pub async fn change_role(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    target: Uuid,
    new_role: TeamRole,
) -> ApiResult<TeamMemberRow> {
    let target = ProfileId::from(target);
    let (mut tx, change) = authorize_member_act(
        pool,
        caller,
        team_id,
        target,
        MembershipAct::Change { role: new_role },
    )
    .await?;
    team_membership::membership_write(&mut tx, None, change).await?;
    let row = member_row(&mut tx, team_id, target).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;
    Ok(row)
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

    /// A profile-owned context shared to `team` via `kb_team_contexts` — the
    /// offboarding shape: a personal context whose resources the team can reach.
    async fn mk_shared_profile_context(pool: &PgPool, owner: Uuid, team: Uuid, slug: &str) -> Uuid {
        let ctx: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
        )
        .bind(owner)
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
            .bind(ctx)
            .bind(team)
            .execute(pool)
            .await
            .unwrap();
        ctx
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

        let detail = team_detail(
            &pool,
            &crate::test_support::caller_for(&pool, owner).await,
            team,
        )
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

        let denied = team_detail(
            &pool,
            &crate::test_support::caller_for(&pool, outsider).await,
            team,
        )
        .await;
        assert!(matches!(denied, Err(ApiError::NotFound(_))));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn remove_member_surfaces_residual_owned_reach(pool: PgPool) {
        // owner removes `leaver`, who owns 2 resources in a shared context + 0 elsewhere.
        let owner = mk_profile(&pool, "owner").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, leaver, "member", "native").await;

        let shared = mk_shared_profile_context(&pool, leaver, team, "shared").await;
        let _r1 = mk_homed_resource(&pool, shared, leaver).await;
        let _r2 = mk_homed_resource(&pool, shared, leaver).await;

        let outcome = remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            leaver,
        )
        .await
        .expect("removal ok");
        assert_eq!(outcome.residual_owned.count, 2);
        assert_eq!(outcome.residual_owned.contexts.len(), 1);
        assert_eq!(outcome.residual_owned.contexts[0].count, 2);
        assert!(outcome.residual_owned.contexts[0]
            .context_ref
            .ends_with("/shared"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn remove_member_residual_groups_same_slug_contexts_by_identity(pool: PgPool) {
        // Two DISTINCT contexts sharing the slug "docs" (slug is unique only per-owner),
        // both shared to the team, both holding leaver-owned resources. Resources are
        // inserted interleaved (A, B, A); kb_resources.id is uuid_generate_v7 (monotonic),
        // so `ORDER BY slug, resource_id` yields the non-adjacent sequence A, B, A. The
        // fold must group by context IDENTITY — an adjacency fold would split A in two.
        let owner = mk_profile(&pool, "owner").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let carol = mk_profile(&pool, "carol").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, leaver, "member", "native").await;

        let ctx_a = mk_shared_profile_context(&pool, carol, team, "docs").await; // @carol/docs
        let ctx_b = mk_shared_profile_context(&pool, leaver, team, "docs").await; // @leaver/docs

        let _r1 = mk_homed_resource(&pool, ctx_a, leaver).await;
        let _r2 = mk_homed_resource(&pool, ctx_b, leaver).await;
        let _r3 = mk_homed_resource(&pool, ctx_a, leaver).await;

        let outcome = remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            leaver,
        )
        .await
        .expect("removal ok");
        assert_eq!(outcome.residual_owned.count, 3);
        assert_eq!(
            outcome.residual_owned.contexts.len(),
            2,
            "one entry per context identity, not per row-run"
        );
        let by_ref: std::collections::HashMap<&str, u32> = outcome
            .residual_owned
            .contexts
            .iter()
            .map(|c| (c.context_ref.as_str(), c.count))
            .collect();
        assert_eq!(by_ref.get("@carol/docs"), Some(&2));
        assert_eq!(by_ref.get("@leaver/docs"), Some(&1));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn remove_member_residual_is_empty_when_nothing_owned(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, leaver, "member", "native").await;

        let outcome = remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            leaver,
        )
        .await
        .expect("removal ok");
        assert_eq!(outcome.residual_owned.count, 0);
        assert!(outcome.residual_owned.contexts.is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn owner_removes_member(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let member = mk_profile(&pool, "member").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, member, "member", "native").await;

        remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            member,
        )
        .await
        .unwrap();
        let detail = team_detail(
            &pool,
            &crate::test_support::caller_for(&pool, owner).await,
            team,
        )
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
        let denied = remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, a).await,
            team,
            b,
        )
        .await;
        assert!(matches!(denied, Err(ApiError::Forbidden)));
        // a removing a (self-leave) → ok.
        remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, a).await,
            team,
            a,
        )
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_remove_last_owner(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        let denied = remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            owner,
        )
        .await;
        assert!(matches!(denied, Err(ApiError::Conflict(_))));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_remove_idp_row(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let idp = mk_profile(&pool, "idp").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, idp, "member", "idp").await;

        let denied = remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            idp,
        )
        .await;
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
            &crate::test_support::human_principal_for(&pool, owner).await,
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

        let denied = change_role(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            member,
            TeamRole::Owner,
        )
        .await;
        assert!(matches!(denied, Err(ApiError::BadRequest(_))));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_demote_last_owner(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        let denied = change_role(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
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

        let denied = change_role(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            ghost,
            TeamRole::Member,
        )
        .await;
        assert!(matches!(denied, Err(ApiError::NotFound(_))));
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
            &crate::test_support::human_principal_for(&pool, owner).await,
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
        remove_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, maintainer).await,
            team,
            member,
        )
        .await
        .unwrap();
        let detail = team_detail(
            &pool,
            &crate::test_support::caller_for(&pool, owner).await,
            team,
        )
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

        // Make `admin` a system admin. Under D11 that is a `kb_principal_governance` grant, not
        // gating-team ownership — `is_system_admin` reads governance. `admin` is NOT a member of
        // `acme`, so this exercises the `is_system_admin` branch of the read gate.
        crate::test_support::grant_governance(&pool, admin).await;

        let detail = team_detail(
            &pool,
            &crate::test_support::caller_for(&pool, admin).await,
            team,
        )
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
        let row = update_team(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            &req,
        )
        .await
        .unwrap();
        assert_eq!(row.name, "Acme Inc");
        assert_eq!(row.description.as_deref(), Some("the roadrunner people"));

        // Partial merge: updating only the name leaves the description intact.
        let req2 = TeamUpdateRequest {
            name: Some("Acme LLC".to_string()),
            description: None,
        };
        let row2 = update_team(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            &req2,
        )
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
        let denied = update_team(
            &pool,
            &crate::test_support::human_principal_for(&pool, member).await,
            team,
            &req,
        )
        .await;
        assert!(matches!(denied, Err(ApiError::Forbidden)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn update_soft_deleted_team_is_not_found(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        delete_team(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
        )
        .await
        .unwrap();
        let req = TeamUpdateRequest {
            name: Some("ghost".to_string()),
            description: None,
        };
        let denied = update_team(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            &req,
        )
        .await;
        assert!(matches!(denied, Err(ApiError::NotFound(_))));
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

        delete_team(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
        )
        .await
        .unwrap();

        // Gone from the caller's listing and no longer showable.
        let after = list_teams(&pool, ProfileId::from(owner)).await.unwrap();
        assert!(!after.iter().any(|t| t.id == team));
        let shown = team_detail(
            &pool,
            &crate::test_support::caller_for(&pool, owner).await,
            team,
        )
        .await;
        assert!(matches!(shown, Err(ApiError::NotFound(_))));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn maintainer_cannot_delete_team(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let maintainer = mk_profile(&pool, "maintainer").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, maintainer, "maintainer", "native").await;

        // A maintainer may manage membership but NOT dissolve the team.
        let denied = delete_team(
            &pool,
            &crate::test_support::human_principal_for(&pool, maintainer).await,
            team,
        )
        .await;
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

        let denied = delete_team(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            sys,
        )
        .await;
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
        delete_team(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
        )
        .await
        .unwrap();
        assert!(!is_visible(&pool, member, resource).await);

        // ...while the resource's own owner still sees it (unaffected by the team).
        assert!(is_visible(&pool, stranger, resource).await);
    }

    // --- `add_member` owner-guard (G3 Phase B2, spec D7) ---

    #[sqlx::test(migrations = "../../migrations")]
    async fn add_member_refuses_to_grant_owner(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let newcomer = mk_profile(&pool, "newcomer").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        let denied = add_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            &AddMemberRequest {
                profile_id: newcomer,
                role: TeamRole::Owner,
            },
        )
        .await;
        assert!(matches!(denied, Err(ApiError::BadRequest(_))));
    }

    /// The `ON CONFLICT DO UPDATE SET role` path is the sneaky one: it upgrades an
    /// EXISTING member to owner, bypassing `change_role`'s guard entirely.
    #[sqlx::test(migrations = "../../migrations")]
    async fn add_member_cannot_upgrade_an_existing_member_to_owner(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let member = mk_profile(&pool, "member").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, member, "member", "native").await;

        let denied = add_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            &AddMemberRequest {
                profile_id: member,
                role: TeamRole::Owner,
            },
        )
        .await;
        assert!(matches!(denied, Err(ApiError::BadRequest(_))));

        // Auth before writes: the refusal leaves the existing row untouched.
        let role: TeamRole = sqlx::query_scalar(
            "SELECT role FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
        )
        .bind(team)
        .bind(member)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(matches!(role, TeamRole::Member), "got {role:?}");
    }

    /// The guard must not break the ordinary path.
    #[sqlx::test(migrations = "../../migrations")]
    async fn add_member_still_adds_a_maintainer(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let newcomer = mk_profile(&pool, "newcomer").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        let row = add_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            &AddMemberRequest {
                profile_id: newcomer,
                role: TeamRole::Maintainer,
            },
        )
        .await
        .unwrap();
        assert!(matches!(row.role, TeamRole::Maintainer));
    }

    /// Read one member's `(role, source)` straight from the table, bypassing the service.
    async fn stored_member(pool: &PgPool, team: Uuid, profile: Uuid) -> (TeamRole, String) {
        sqlx::query_as(
            "SELECT role, source::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
        )
        .bind(team)
        .bind(profile)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The orphaning path. Under the old upsert a maintainer could "add" the team's SOLE owner
    /// at `watcher`, overwriting the role with no last-owner guard and leaving the team with no
    /// owner at all. `change_role` refuses that demotion; `add_member` must not be a way around it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn add_member_cannot_demote_the_last_owner(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let maintainer = mk_profile(&pool, "maintainer").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, maintainer, "maintainer", "native").await;

        let denied = add_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, maintainer).await,
            team,
            &AddMemberRequest {
                profile_id: owner,
                role: TeamRole::Watcher,
            },
        )
        .await;
        assert!(
            matches!(denied, Err(ApiError::Conflict(_))),
            "got {denied:?}"
        );

        let (role, _) = stored_member(&pool, team, owner).await;
        assert!(matches!(role, TeamRole::Owner), "got {role:?}");
    }

    /// A SAML-provisioned row is changed only by IdP reconciliation; `change_role` refuses it.
    /// The old upsert rewrote its role and left `source = 'idp'` on a row the IdP never asserted.
    #[sqlx::test(migrations = "../../migrations")]
    async fn add_member_cannot_rewrite_a_saml_provisioned_row(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let provisioned = mk_profile(&pool, "provisioned").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, provisioned, "member", "idp").await;

        let denied = add_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            &AddMemberRequest {
                profile_id: provisioned,
                role: TeamRole::Maintainer,
            },
        )
        .await;
        assert!(
            matches!(denied, Err(ApiError::Conflict(_))),
            "got {denied:?}"
        );

        let (role, source) = stored_member(&pool, team, provisioned).await;
        assert!(matches!(role, TeamRole::Member), "got {role:?}");
        assert_eq!(source, "idp");
    }

    /// The ordinary case of the same rule: re-adding an existing native member at a different
    /// role is refused, not applied — the role changes only through `change_role`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn add_member_does_not_change_an_existing_members_role(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let member = mk_profile(&pool, "member").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, member, "member", "native").await;

        let denied = add_member(
            &pool,
            &crate::test_support::human_principal_for(&pool, owner).await,
            team,
            &AddMemberRequest {
                profile_id: member,
                role: TeamRole::Maintainer,
            },
        )
        .await;
        assert!(
            matches!(denied, Err(ApiError::Conflict(_))),
            "got {denied:?}"
        );

        let (role, _) = stored_member(&pool, team, member).await;
        assert!(matches!(role, TeamRole::Member), "got {role:?}");
    }

    // --- Personal-team slug squatting (spec review L1) ---

    /// Insert a profile and return the database's answer, so a refused insert can be asserted.
    async fn try_mk_profile(pool: &PgPool, handle: &str) -> Result<Uuid, sqlx::Error> {
        sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
    }

    /// Nobody may create a team under the personal prefix, admins included, in any case.
    #[sqlx::test(migrations = "../../migrations")]
    async fn create_team_refuses_the_personal_slug_prefix(pool: PgPool) {
        let squatter = mk_profile(&pool, "squatter").await;
        let authed = crate::test_support::human_principal_for(&pool, squatter).await;
        for slug in ["personal-victim", "Personal-Victim"] {
            let req = TeamCreateRequest {
                slug: slug.to_string(),
                name: None,
                parent: None,
                auto_join_role: None,
            };
            let refused = create_team(&pool, &authed, &req).await;
            assert!(
                matches!(refused, Err(ApiError::BadRequest(_))),
                "{slug}: got {refused:?}"
            );
        }
        let held: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM kb_teams WHERE lower(slug) = 'personal-victim')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!held, "a refused create must write no team");
    }

    /// The profile's own personal team, found by identity.
    async fn personal_team_of(pool: &PgPool, profile: Uuid) -> (Uuid, String) {
        sqlx::query_as("SELECT id, slug FROM kb_teams WHERE personal_of = $1")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn member_count(pool: &PgPool, team: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM kb_team_members WHERE team_id = $1")
            .bind(team)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// A team already holding `personal-<handle>` with a member is someone else's: the new
    /// profile is not joined to it, and still gets a personal team, at the next free suffix.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_new_profile_does_not_join_a_member_held_personal_slug(pool: PgPool) {
        let squatter = mk_profile(&pool, "squatter").await;
        let squat = mk_team(&pool, "personal-victim").await;
        add(&pool, squat, squatter, "owner", "native").await;

        let victim = try_mk_profile(&pool, "victim")
            .await
            .expect("a held personal slug never blocks a profile");

        assert_eq!(
            member_count(&pool, squat).await,
            1,
            "the squatted team gains no member"
        );
        let (own, slug) = personal_team_of(&pool, victim).await;
        assert_eq!(slug, "personal-victim-2");
        let (role, _) = stored_member(&pool, own, victim).await;
        assert!(matches!(role, TeamRole::Owner), "got {role:?}");
        assert_eq!(member_count(&pool, own).await, 1);
    }

    /// A team someone else built under the slug and its `-2` are both passed over: the suffix
    /// takes the next free one.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_new_profile_skips_every_held_suffix(pool: PgPool) {
        let parent = mk_team(&pool, "acme").await;
        let squat = mk_team(&pool, "personal-victim").await;
        sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
            .bind(squat)
            .bind(parent)
            .execute(&pool)
            .await
            .unwrap();
        let squatter = mk_profile(&pool, "squatter").await;
        let next = mk_team(&pool, "personal-victim-2").await;
        add(&pool, next, squatter, "owner", "native").await;

        let victim = try_mk_profile(&pool, "victim").await.unwrap();

        assert_eq!(member_count(&pool, squat).await, 0);
        let (_, slug) = personal_team_of(&pool, victim).await;
        assert_eq!(slug, "personal-victim-3");
    }

    /// An empty team at the bare slug is still someone else's: whatever hangs off it (here a
    /// child team, whose members reach their ancestors) must not become the new profile's.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_new_profile_does_not_adopt_a_memberless_team_at_its_slug(pool: PgPool) {
        let empty = mk_team(&pool, "personal-victim").await;
        let child = mk_team(&pool, "foothold").await;
        sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
            .bind(child)
            .bind(empty)
            .execute(&pool)
            .await
            .unwrap();

        let victim = try_mk_profile(&pool, "victim").await.unwrap();

        assert_eq!(
            member_count(&pool, empty).await,
            0,
            "the empty team gains no member"
        );
        let (_, slug) = personal_team_of(&pool, victim).await;
        assert_eq!(slug, "personal-victim-2");
    }

    /// Replay restores every personal team, carrying `personal_of`, before its profile, under
    /// whatever slug genesis gave it, suffixed or renamed by erasure; the profile adopts it by
    /// identity and creates nothing.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_restored_personal_team_is_adopted_by_identity_whatever_its_slug(pool: PgPool) {
        let id = Uuid::now_v7();
        let restored: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name, personal_of) \
             VALUES ('personal-ghost-2', 'g', $1) RETURNING id",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, 'ghost', 'g')")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(personal_team_of(&pool, id).await.0, restored);
        let teams: i64 =
            sqlx::query_scalar("SELECT count(*) FROM kb_teams WHERE slug LIKE 'personal-ghost%'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(teams, 1, "no second personal team is created");
    }
}
