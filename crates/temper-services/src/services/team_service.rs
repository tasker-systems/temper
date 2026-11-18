//! Team lifecycle service over the substrate.
//!
//! Service-direct, NO `DbBackend` command — teams are provisioning/infrastructure, not
//! knowledge-graph content (org-provisioning spec §2.6, the same precedent as `context_service`).
//! No cognition events. Membership acts are the exception that proves the rule: `add_member`,
//! `change_role` and `remove_member` are authority acts, decided by the rank rule in
//! `authz::team_membership` and recorded on the ADMIN ledger (`team_membership_changed`, NULL-
//! anchored), atomically with the row (team-role rank spec C8).
//!
//! Role-gating is pure authz over `kb_team_members.role` + the `kb_teams_parents`
//! DAG (spec §3 decision #1): anyone may create a **root** (parentless) team and
//! becomes its `owner`; creating a **child** requires `owner`/`maintainer` on the
//! parent (a system admin, when the parent is the root or the gating team; never under a
//! personal team); setting `auto_join_role` requires `is_system_admin`. Auth checks
//! precede every write.

use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::{Caller, HumanPrincipal, SystemAdmin};
use crate::authz::team_membership::{self, MembershipAct, MembershipAuthority};
use crate::error::{ApiError, ApiResult};
use crate::services::saml_mapping_service;
use temper_core::types::ids::ProfileId;
use temper_core::types::reassign::{RemoveMemberOutcome, ResidualContext, ResidualOwnedReach};
use temper_core::types::team::{
    AddMemberRequest, AttachChildOutcome, AttachOutcome, PublicWithinAudience, PublicWithinEffects,
    PublicWithinInRef, PublicWithinOpenKind, PublicWithinOutcome, PublicWithinPolicyOutcome,
    PublicWithinReach, PublicWithinRef, PublicWithinRowRef, RenameTeamSlugOutcome,
    RenameTeamSlugRequest, ReparentOutcome, ReparentRequest, RestoreTeamOutcome,
    SetAutoJoinOutcome, TeamChildRef, TeamCreateRequest, TeamDetail, TeamMemberDetail,
    TeamMemberRow, TeamMemberSource, TeamRef, TeamRole, TeamRow, TeamUpdateRequest,
    MAX_CREATE_PARENTS,
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

/// The caller's role on a **live** team, if any: no row on a soft-deleted team.
///
/// `pub(crate)` so sibling services (e.g. `context_service`'s team-owned-context
/// gate) reuse the one role check rather than duplicating the authz.
///
/// **Live-only, so that a new caller is safe by default.** `20260703000001_team_metadata_soft_delete.sql`
/// says membership in a soft-deleted team confers nothing anywhere. This probe used to be
/// `is_active`-blind and left each caller to gate the team, and several authority callers did not
/// (a former manager kept administering a dead team's contexts, connections and machine clients).
/// Every authority decision asks this one. The blind lookup is [`membership_row_role`], for the
/// two callers that must tell an already-deleted team from a non-member.
pub(crate) async fn role_on_team(
    pool: &PgPool,
    team_id: Uuid,
    profile_id: ProfileId,
) -> ApiResult<Option<TeamRole>> {
    let mut conn = pool.acquire().await?;
    role_on_team_on(&mut conn, team_id, profile_id).await
}

/// [`role_on_team`] on a connection — for a decision made inside a write transaction, under its
/// locks (team-role rank plan P1). The one query both spellings run, equally live-only.
pub(crate) async fn role_on_team_on(
    conn: &mut sqlx::PgConnection,
    team_id: Uuid,
    profile_id: ProfileId,
) -> ApiResult<Option<TeamRole>> {
    let role = sqlx::query_scalar!(
        r#"SELECT tm.role AS "role: TeamRole"
             FROM kb_team_members tm
             JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
            WHERE tm.team_id = $1 AND tm.profile_id = $2"#,
        team_id,
        *profile_id,
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(role)
}

/// The caller's membership row on a team, **live or not**. Not an authority answer: a role on a
/// soft-deleted team confers nothing. Its only callers are [`update_team`] and [`delete_team`],
/// which pass it to tell a team that is already deleted (their `is_active` write answers
/// `NotFound`) from one the caller is not on (`Forbidden`). An authority decision asks
/// [`role_on_team`].
async fn membership_row_role<'e, E: sqlx::PgExecutor<'e>>(
    pool: E,
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

/// The refusal for anything that would put someone else on a personal team, or hang a team under
/// one (task `01a19006`). The database holds the same rule (`personal_team_admits` and its
/// triggers); this is the refusal by name.
pub(crate) const PERSONAL_TEAM_HOLDS_ITS_OWNER: &str =
    "a personal team holds only its owner and the owner's machines";

/// Whose personal team this is, or `None` for a shared team (or no team).
///
/// A personal team is acted on by its owner alone, and by erasure: a system admin's arm does not
/// reach it (ruled 2026-10-31). Every gate that would otherwise let an admin through asks this
/// first. Listing a personal team for audit is not an act, and is not gated by it.
pub(crate) async fn personal_of(pool: &PgPool, team_id: Uuid) -> ApiResult<Option<ProfileId>> {
    let mut conn = pool.acquire().await?;
    personal_of_on(&mut conn, team_id).await
}

/// [`personal_of`] on a connection — for a decision made inside a write transaction, under its
/// locks.
pub(crate) async fn personal_of_on(
    conn: &mut sqlx::PgConnection,
    team_id: Uuid,
) -> ApiResult<Option<ProfileId>> {
    let owner = sqlx::query_scalar!("SELECT personal_of FROM kb_teams WHERE id = $1", team_id)
        .fetch_optional(&mut *conn)
        .await?
        .flatten();
    Ok(owner.map(ProfileId::from))
}

/// The person whose personal space a team's holdings are: [`personal_of`] when that profile is a
/// person, `None` for a shared team or an agent's personal team (`profile_is_person`, migration
/// 20261113000000). The admin arm over machines, connections and subscriptions stops here; an
/// agent's personal team is not anyone's personal space, so an admin still reaches it.
pub(crate) async fn personal_space_owner(
    pool: &PgPool,
    team_id: Uuid,
) -> ApiResult<Option<ProfileId>> {
    let owner = sqlx::query_scalar!(
        r#"SELECT CASE WHEN profile_is_person(personal_of) THEN personal_of END AS "owner"
             FROM kb_teams WHERE id = $1"#,
        team_id
    )
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(owner.map(ProfileId::from))
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
/// also refuses a non-admin on the gating team and temper-system (team-role rank spec R11). `machine_authz::contain_reach`
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

/// Assert a **live** team row exists, for callers whose authority skips [`require_manage_on_team`] (a
/// system admin) and who would otherwise write an unvalidated `principal_id`. A soft-deleted team is
/// refused too: nothing is written onto a dead team, admins included (soft-delete design D4). `kb_access_grants`
/// carries no FK on `principal_id`, so this is the only thing standing between an admin's typo
/// and a dangling grant nobody can see or revoke by team name.
pub(crate) async fn require_team_exists(pool: &PgPool, team_id: Uuid) -> ApiResult<()> {
    let exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_teams WHERE id = $1 AND is_active) AS "e!: bool""#,
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

/// Is `team_id` the root (`temper-system`) or the configured gating team? The lifecycle gates
/// (`create_team` as parent, `update_team`, `delete_team`) are admin-only on both (team-authority
/// fixes D4). The predicate is [`access_service::is_root_or_gating_team`], which every
/// instance-team gate shares.
///
/// [`access_service::is_root_or_gating_team`]: crate::services::access_service::is_root_or_gating_team
async fn is_root_or_gating(pool: &PgPool, team_id: Uuid) -> ApiResult<bool> {
    crate::services::access_service::is_root_or_gating_team(pool, team_id).await
}

/// Refuse unless `caller` is a system admin.
async fn require_admin_on(pool: &PgPool, caller: &HumanPrincipal) -> ApiResult<()> {
    match crate::authz::Principal::Human(caller)
        .system_admin(pool)
        .await?
    {
        Some(_) => Ok(()),
        None => Err(ApiError::Forbidden),
    }
}

/// [`require_admin_on`] on the transaction's own connection, for a re-check under row locks: a
/// transaction that holds rows never waits on a second pooled connection (which a saturated pool
/// may never hand back). A demotion serializes with it (`auth::require_system_admin_locked`).
async fn require_admin_in(conn: &mut sqlx::PgConnection, caller: &HumanPrincipal) -> ApiResult<()> {
    crate::auth::require_system_admin_locked(conn, caller)
        .await
        .map(|_| ())
}

/// A `kb_teams` row as the SQL selects it. [`TeamRow`] is the wire answer, which also carries the
/// opt-in `public_within` fields a create fills; a row read from the table carries neither.
pub(crate) struct TeamRowDb {
    id: Uuid,
    slug: String,
    name: String,
    description: Option<String>,
    created: chrono::DateTime<chrono::Utc>,
    auto_join_role: Option<TeamRole>,
}

impl From<TeamRowDb> for TeamRow {
    fn from(r: TeamRowDb) -> Self {
        TeamRow {
            id: r.id,
            slug: r.slug,
            name: r.name,
            description: r.description,
            created: r.created,
            auto_join_role: r.auto_join_role,
            public_within: None,
            public_within_in_reach: None,
            elevated_mappings: None,
        }
    }
}

/// One `public_within` row named by its two team ids, as a transitions answer carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub(crate) struct RowKey {
    team_id: Uuid,
    anchor_id: Uuid,
}

/// What `_public_within_transitions` (migration `20261122000000`) answered for one act, unfiltered:
/// the rows deleted because the anchor is no longer above the team by any path, and the rows whose
/// effect flipped. [`name_effects`] filters it for the caller.
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct ActEffects {
    #[serde(default)]
    lapsed: Vec<RowKey>,
    #[serde(default)]
    changes: Vec<EffectChange>,
}

impl ActEffects {
    /// Fold a later write's effects of the same act into this one. A row that flipped twice in one
    /// act (once per link of a several-parent create) keeps only its last state, and a row that
    /// flipped back is dropped; a lapsed row stays lapsed.
    fn absorb(&mut self, later: ActEffects) {
        self.lapsed.extend(later.lapsed);
        for change in later.changes {
            let key = (change.team_id, change.anchor_id);
            match self
                .changes
                .iter()
                .position(|c| (c.team_id, c.anchor_id) == key)
            {
                Some(i) if self.changes[i].effective != change.effective => {
                    self.changes.remove(i);
                }
                Some(_) => {}
                None => self.changes.push(change),
            }
        }
        let lapsed: Vec<(Uuid, Uuid)> = self
            .lapsed
            .iter()
            .map(|l| (l.team_id, l.anchor_id))
            .collect();
        self.changes
            .retain(|c| !lapsed.contains(&(c.team_id, c.anchor_id)));
    }
}

/// The answer of `_admin_team_parent_link_act` and `_admin_team_lifecycle_act`.
#[derive(Debug, serde::Deserialize)]
struct ActWrite {
    outcome: String,
    #[serde(flatten)]
    effects: ActEffects,
}

/// Does `me` see a `public_within` row named on an act's answer? A system admin, a direct, live
/// manager of the row's team, or a direct, live manager of its anchor when the anchor is not the root
/// or the gating team (a leftover managing row there confers nothing). Everyone else gets a count.
async fn names_row(
    conn: &mut sqlx::PgConnection,
    admin: bool,
    me: ProfileId,
    team_id: Uuid,
    anchor_id: Uuid,
) -> ApiResult<bool> {
    names_row_owning(conn, admin, me, None, team_id, anchor_id).await
}

/// [`names_row`], where `owned` is a team the act's own authority already found the caller managing
/// before the act wrote: a delete makes its team dead, and a dead team's rows confer no live
/// standing, but its owner is still a manager of the row's side for the answer to that delete.
async fn names_row_owning(
    conn: &mut sqlx::PgConnection,
    admin: bool,
    me: ProfileId,
    owned: Option<Uuid>,
    team_id: Uuid,
    anchor_id: Uuid,
) -> ApiResult<bool> {
    let owns = |t: Uuid| owned == Some(t);
    Ok(admin
        || owns(team_id)
        || (owns(anchor_id)
            && !crate::services::access_service::is_root_or_gating_team_on(conn, anchor_id)
                .await?)
        || manages_on(conn, team_id, me).await?
        || (manages_on(conn, anchor_id, me).await?
            && !crate::services::access_service::is_root_or_gating_team_on(conn, anchor_id)
                .await?))
}

async fn row_ref(
    conn: &mut sqlx::PgConnection,
    team_id: Uuid,
    anchor_id: Uuid,
) -> ApiResult<PublicWithinRowRef> {
    Ok(PublicWithinRowRef {
        team: team_ref_of(conn, team_id).await?,
        anchor: team_ref_of(conn, anchor_id).await?,
    })
}

/// Filter an act's `public_within` effects for its caller: each row named to a caller [`names_row`]
/// admits, and counted otherwise. Run in the act's transaction, after its write: the caller's
/// standing is read there (the act locks the teams it touched, not every row's team and anchor).
async fn name_effects(
    conn: &mut sqlx::PgConnection,
    caller: &HumanPrincipal,
    effects: ActEffects,
) -> ApiResult<PublicWithinEffects> {
    name_effects_owning(conn, caller, None, effects).await
}

/// [`name_effects`], with `owned` as in [`names_row_owning`].
async fn name_effects_owning(
    conn: &mut sqlx::PgConnection,
    caller: &HumanPrincipal,
    owned: Option<Uuid>,
    effects: ActEffects,
) -> ApiResult<PublicWithinEffects> {
    let mut out = PublicWithinEffects::default();
    if effects.lapsed.is_empty() && effects.changes.is_empty() {
        return Ok(out);
    }
    let admin = caller_is_admin_on(conn, caller).await?;
    let me = caller.profile_id();
    for row in effects.lapsed {
        if names_row_owning(conn, admin, me, owned, row.team_id, row.anchor_id).await? {
            out.lapsed
                .push(row_ref(conn, row.team_id, row.anchor_id).await?);
        } else {
            out.lapsed_unnamed += 1;
        }
    }
    for change in effects.changes {
        let named =
            names_row_owning(conn, admin, me, owned, change.team_id, change.anchor_id).await?;
        match (named, change.effective) {
            (true, true) => out
                .revived
                .push(row_ref(conn, change.team_id, change.anchor_id).await?),
            (true, false) => out
                .dormant
                .push(row_ref(conn, change.team_id, change.anchor_id).await?),
            (false, true) => out.revived_unnamed += 1,
            (false, false) => out.dormant_unnamed += 1,
        }
    }
    Ok(out)
}

/// The effective `public_within` rows anchored on `team_id` or one of its live ancestors: the rows
/// the team's roster reads through. Read under the effects lock, so a before/after pair around a
/// link is not moved by a concurrent open.
async fn rows_in_reach(conn: &mut sqlx::PgConnection, team_id: Uuid) -> ApiResult<Vec<RowKey>> {
    Ok(sqlx::query!(
        r#"SELECT pw.team_id, pw.anchor_id
             FROM kb_team_public_within pw
            WHERE pw.anchor_id IN (SELECT a.team_id FROM team_ancestors($1) a)
              AND public_within_row_effective(pw.team_id, pw.anchor_id, pw.instance_wide)
            ORDER BY pw.team_id, pw.anchor_id"#,
        team_id,
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|r| RowKey {
        team_id: r.team_id,
        anchor_id: r.anchor_id,
    })
    .collect())
}

/// Take `_public_within_effects_lock` now, so that a read made before an act's write and the act's
/// own transitions see the same rows. Callers hold every team row they will lock first.
async fn effects_lock(conn: &mut sqlx::PgConnection) -> ApiResult<()> {
    sqlx::query_scalar!(r#"SELECT 1 AS "one!" FROM (SELECT _public_within_effects_lock()) l"#)
        .fetch_one(&mut *conn)
        .await?;
    Ok(())
}

/// The rows in `after` and not in `before`, filtered for the caller as [`name_effects`] does.
async fn name_reach(
    conn: &mut sqlx::PgConnection,
    caller: &HumanPrincipal,
    before: &[RowKey],
    after: Vec<RowKey>,
) -> ApiResult<PublicWithinReach> {
    let mut out = PublicWithinReach::default();
    let new: Vec<RowKey> = after.into_iter().filter(|r| !before.contains(r)).collect();
    if new.is_empty() {
        return Ok(out);
    }
    let admin = caller_is_admin_on(conn, caller).await?;
    let me = caller.profile_id();
    for row in new {
        if names_row(conn, admin, me, row.team_id, row.anchor_id).await? {
            out.rows
                .push(row_ref(conn, row.team_id, row.anchor_id).await?);
        } else {
            out.unnamed += 1;
        }
    }
    Ok(out)
}

/// What [`lifecycle_write`] did: the act was written and recorded, it asked for the value already
/// held (nothing written), there was no team in the state the act needs (nothing written), or it
/// was a delete of the instance's gating team, which is never deleted (nothing written; migration
/// `20261124000050`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LifecycleOutcome {
    Changed,
    Noop,
    Absent,
    Gating,
}

/// Perform one team lifecycle act through `_admin_team_lifecycle_act` (migration
/// `20261122000000`): the `kb_teams` change, its `team_lifecycle_changed` record, and the record of
/// every `public_within` row whose effect it flipped, in one statement. `act` is one of `created`,
/// `deleted`, `restored`, `renamed`, `auto_join_changed`; `to` is the new slug or role for the last
/// two. Answers the outcome and the unfiltered effects. It NEVER AUTHORIZES — every caller decides
/// first.
pub(crate) async fn lifecycle_write(
    tx: &mut sqlx::PgConnection,
    emitter: Uuid,
    actor: ProfileId,
    team_id: Uuid,
    act: &str,
    to: Option<&str>,
    correlation: Option<Uuid>,
) -> ApiResult<(LifecycleOutcome, ActEffects)> {
    let raw = sqlx::query_scalar!(
        r#"SELECT _admin_team_lifecycle_act($1, $2, $3, $4, $5, $6) AS "w!""#,
        emitter,
        *actor,
        team_id,
        act,
        to,
        correlation,
    )
    .fetch_one(&mut *tx)
    .await?;
    let write: ActWrite = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("_admin_team_lifecycle_act: {e}")))?;
    let outcome = match write.outcome.as_str() {
        "changed" => LifecycleOutcome::Changed,
        "noop" => LifecycleOutcome::Noop,
        "absent" => LifecycleOutcome::Absent,
        "gating" => LifecycleOutcome::Gating,
        other => {
            return Err(ApiError::Internal(format!(
                "_admin_team_lifecycle_act answered {other}"
            )))
        }
    };
    Ok((outcome, write.effects))
}

/// Proof that `create_team` has just inserted the team row it is about to write the creator's
/// `owner` row for. Defined here with a private field and constructed only in this module, so the
/// genesis constructor in `authz::team_membership` cannot be called from anywhere else (plan P16).
#[derive(Debug)]
pub(crate) struct GenesisToken {
    _private: (),
}

/// Create a team. The caller becomes its `owner`.
///
/// The [`PERSONAL_TEAM_SLUG_PREFIX`] is refused for every caller, admins included: only the
/// `sync_personal_team` trigger creates a personal team. The trigger no longer joins a profile to
/// a team it did not create (a held slug sends it to `personal-<handle>-N`), so the refusal is the
/// first of two guards against squatting, and keeps `personal-<handle>` meaning that profile's.
///
/// Auth before writes:
/// - **child** (`parent` set): caller must be `owner`/`maintainer` on the parent. When the parent is
///   the root or the gating team, a system admin only. A personal team takes no children (400).
/// - **root** (`parent` None): any authenticated profile may create.
/// - `auto_join_role` set: caller must be `is_system_admin`, and the role must be `member` or
///   `watcher` (400 otherwise; spec C15).
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
    Ok(create_team_with_effects(pool, authed, req).await?.team)
}

/// What [`create_team_with_effects`] did: the new team's row, what its parent links did to
/// `public_within` rows (an auto-join team created under an anchor makes its rows dormant), and the
/// effective rows the new team's roster reads through its parents. Both filtered for the caller.
/// `POST /api/teams` answers the row alone unless asked with `?answer=effects`, which fills
/// `TeamRow.public_within` and `public_within_in_reach` from this; the transitions are on the admin
/// ledger either way.
#[derive(Debug, Clone)]
pub struct CreatedTeam {
    pub team: TeamRow,
    pub public_within: PublicWithinEffects,
    pub public_within_in_reach: PublicWithinReach,
    /// The SAML group mappings the create elevated (an auto-join team under a parent), each
    /// recorded once as `saml_group_mapping_changed`, act `elevated`.
    pub elevated_mappings: Vec<temper_core::types::saml_mapping::SamlGroupMapping>,
}

/// [`create_team`], answering what the create did to `public_within` rows as well.
pub async fn create_team_with_effects(
    pool: &PgPool,
    authed: &HumanPrincipal,
    req: &TeamCreateRequest,
) -> ApiResult<CreatedTeam> {
    let creator = ProfileId::from(authed.profile().id);
    // One validator for new slugs and renames (ruling R4); it refuses the personal prefix too.
    let slug = validate_team_slug(&req.slug)?;

    // A team's creator becomes its `owner`, and a machine principal never governs (ruled
    // 2026-10-09; the database refuses the row — 20261019100000_machines_never_govern.sql). The
    // `&HumanPrincipal` parameter is what keeps a machine out: the classification that minted it
    // already read `is_machine_profile`.

    // --- Auth before writes ---

    // Child team: resolve each parent and require owner/maintainer on it (team DAG doors D4: the
    // several-parent form is checked exactly as the one parent always was). This is the pool
    // pre-check; the same decision runs again under the parents' row locks before any link is
    // written.
    let parent_ids = resolve_create_parents(pool, req).await?;
    for &parent_id in &parent_ids {
        // The root and the gating team take children only from a system admin (team-authority
        // fixes D4): a child of either is read reach into everything attached to it, and R11 keeps
        // the gating roster admin-only. A leftover managing row (a demoted admin's) confers nothing.
        if is_root_or_gating(pool, parent_id).await? {
            require_admin_on(pool, authed).await?;
        } else {
            match role_on_team(pool, parent_id, creator).await? {
                Some(role) if can_manage(role) => {}
                _ => return Err(ApiError::Forbidden),
            }
            // After the role check, so a non-manager learns nothing about the parent.
            if personal_of(pool, parent_id).await?.is_some() {
                return Err(ApiError::BadRequest(
                    "a personal team cannot have child teams".to_string(),
                ));
            }
        }
    }

    // auto_join_role defines an everyone-pool — admin-gated.
    if req.auto_join_role.is_some()
        && crate::authz::Principal::Human(authed)
            .system_admin(pool)
            .await?
            .is_none()
    {
        return Err(ApiError::Forbidden);
    }
    // ...and is never a role that governs the team (spec C15): `owner`/`maintainer` is granted by
    // a person. After the admin gate, so a non-admin learns nothing from it; before any write, so
    // the `kb_teams_auto_join_role_at_most_member` CHECK never surfaces as a 500.
    if req
        .auto_join_role
        .is_some_and(|role| role.rank() > TeamRole::Member.rank())
    {
        return Err(ApiError::BadRequest(
            "auto_join_role may be member or watcher; a role that governs a team is granted by a \
             person"
                .to_string(),
        ));
    }

    // --- Writes (one transaction) ---
    let team_id = Uuid::now_v7();
    let name = req.name.clone().unwrap_or_else(|| slug.clone());

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    // Under the parents' row locks (the one every membership writer takes), taken in ascending id
    // order, the pre-check's decision is made again for each: the parent is still live, and either
    // it is an instance team and the caller is a system admin, or it is not and the creator still
    // manages it. A concurrent demotion, removal, soft-delete or re-pointed gating slug cannot race
    // the link it would have refused (team DAG review G12). Same pattern as `detach_child`.
    lock_teams_ascending(&mut tx, &parent_ids).await?;
    for &parent_id in &parent_ids {
        if !link_side(&mut tx, parent_id)
            .await?
            .is_some_and(|side| side.live)
        {
            return Err(ApiError::NotFound(
                "parent team not found or not readable".to_string(),
            ));
        }
        if crate::services::access_service::is_root_or_gating_team_on(&mut tx, parent_id).await? {
            require_admin_in(&mut tx, authed).await?;
        } else if !matches!(
            role_on_team_on(&mut tx, parent_id, creator).await?,
            Some(role) if can_manage(role)
        ) {
            return Err(ApiError::Forbidden);
        }
    }
    // The auto-join gate, asked again on the transaction: a demotion committed since the pool
    // check refuses, and one still in flight waits for this create.
    if req.auto_join_role.is_some() {
        require_admin_in(&mut tx, authed).await?;
    }
    // An auto-join team created under a parent is a pool beneath the parent and everything above
    // it: the SAML mappings over a pool, read before the links, after the parents' row locks.
    // Only an auto-join team (a system admin's create) is a pool; a plain child team elevates
    // nothing, so the scan runs on that arm alone.
    let elevation = if parent_ids.is_empty() || req.auto_join_role.is_none() {
        None
    } else {
        Some(saml_mapping_service::elevation_before(&mut tx, &parent_ids).await?)
    };

    let row = sqlx::query_as!(
        TeamRowDb,
        r#"
        INSERT INTO kb_teams (id, slug, name, auto_join_role)
        VALUES ($1, $2, $3, $4)
        RETURNING id, slug, name, description, created,
                  auto_join_role AS "auto_join_role: TeamRole"
        "#,
        team_id,
        slug,
        name,
        req.auto_join_role as Option<TeamRole>,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| map_unique_violation(e, "team slug already exists"))?;

    // Each parent link is read reach into everything attached to the parent, so it is a recorded
    // act (team-authority fixes D3): the row and its `team_parent_link_changed` record are one
    // write. The links of one create share a correlation id (D4), and each records the
    // `public_within` rows it flips (an auto-join team created under an anchor makes its rows
    // dormant).
    let mut effects = ActEffects::default();
    let mut elevated_mappings = Vec::new();
    if !parent_ids.is_empty() {
        let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, creator, "web")
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let correlation = Uuid::now_v7();
        for &parent_id in &parent_ids {
            let (_, linked) = link(
                &mut tx,
                emitter.uuid(),
                creator,
                parent_id,
                team_id,
                Some(correlation),
            )
            .await?;
            effects.absorb(linked);
        }
        elevated_mappings = saml_mapping_service::record_elevations(
            &mut tx,
            elevation,
            emitter.uuid(),
            creator,
            Some(correlation),
        )
        .await?;
    }
    let public_within = name_effects(&mut tx, authed, effects).await?;
    let in_reach = if parent_ids.is_empty() {
        PublicWithinReach::default()
    } else {
        let after = rows_in_reach(&mut tx, team_id).await?;
        name_reach(&mut tx, authed, &[], after).await?
    };

    // The creator's `owner` row is a recorded act (plan P7): it goes through the one membership
    // writer with `via = team_genesis`, in the same transaction as the team insert. `locked` is not
    // run first — the team was born in this transaction, so no concurrent writer can see it, and
    // the SQL writer re-takes both locks itself.
    team_membership::membership_write(
        &mut tx,
        None,
        team_membership::AuthorizedMembershipChange::genesis(
            GenesisToken { _private: () },
            team_id,
            creator,
        ),
    )
    .await?;

    // The team itself is a recorded act (lifecycle doors D8, R5 extended), in the insert's
    // transaction: the writer's `created` act appends only.
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, creator, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    lifecycle_write(
        &mut tx,
        emitter.uuid(),
        creator,
        team_id,
        "created",
        None,
        None,
    )
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

    Ok(CreatedTeam {
        team: row.into(),
        public_within,
        public_within_in_reach: in_reach,
        elevated_mappings,
    })
}

/// The one team-slug validator (team DAG doors ruling R4), for every new slug — a create and a
/// rename. Lowercases its input, then requires `^[a-z0-9][a-z0-9-]{0,62}$` and refuses the
/// [`PERSONAL_TEAM_SLUG_PREFIX`] (only `sync_personal_team` mints one). Existing slugs are
/// grandfathered: nothing re-validates a slug already stored. Returns the slug to store.
pub fn validate_team_slug(raw: &str) -> ApiResult<String> {
    let slug = raw.to_ascii_lowercase();
    let mut chars = slug.chars();
    let shaped = slug.len() <= 63
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !shaped {
        return Err(ApiError::BadRequest(
            "a team slug is 1-63 characters of a-z, 0-9 and `-`, starting with a letter or digit"
                .to_string(),
        ));
    }
    if slug.starts_with(PERSONAL_TEAM_SLUG_PREFIX) {
        return Err(ApiError::BadRequest(format!(
            "the `{PERSONAL_TEAM_SLUG_PREFIX}` slug prefix is reserved for personal teams"
        )));
    }
    Ok(slug)
}

/// The parents a create names (team DAG doors D4), resolved to live team ids in request order with
/// duplicates collapsed: `parent` is the one-element form of `parents`, and sending both is `400`;
/// more than [`MAX_CREATE_PARENTS`] is `400`; a ref naming no live team is `404`.
async fn resolve_create_parents(pool: &PgPool, req: &TeamCreateRequest) -> ApiResult<Vec<Uuid>> {
    let refs: Vec<&str> = match (&req.parent, &req.parents) {
        (Some(_), Some(_)) => {
            return Err(ApiError::BadRequest(
                "send `parent` or `parents`, not both".to_string(),
            ))
        }
        (Some(one), None) => vec![one.as_str()],
        (None, Some(many)) => many.iter().map(String::as_str).collect(),
        (None, None) => Vec::new(),
    };
    if refs.len() > MAX_CREATE_PARENTS {
        return Err(ApiError::BadRequest(format!(
            "a team is created with at most {MAX_CREATE_PARENTS} parents"
        )));
    }
    let mut ids: Vec<Uuid> = Vec::with_capacity(refs.len());
    for parent_ref in refs {
        let live = match resolve_team_ref(pool, parent_ref).await? {
            Some(id) => sqlx::query_scalar!(
                r#"SELECT is_active AS "live!" FROM kb_teams WHERE id = $1"#,
                id
            )
            .fetch_one(pool)
            .await?
            .then_some(id),
            None => None,
        };
        let id = live.ok_or_else(|| {
            ApiError::NotFound("parent team not found or not readable".to_string())
        })?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Add a member to a team at `req.role` — the rank rule (team-role rank spec C1, R11): a manager
/// can grant a role up to their own (an owner can grant `owner`, a maintainer up to `maintainer`);
/// only a system admin can add to the gating team or temper-system, and a system admin can grant any role. A
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
        MembershipAuthority::resolve_precheck(&mut conn, caller, team_id)
            .await?
            .precheck(act, target)?;
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;
    let locked = team_membership::locked(&mut tx, team_id, target).await?;
    let resolved = MembershipAuthority::resolve(&mut tx, caller, team_id).await?;
    let change = team_membership::authorize(resolved, act, &locked)?;
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
        TeamRowDb,
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

    Ok(rows.into_iter().map(TeamRow::from).collect())
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
        TeamRowDb,
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

    // The direct children, for a caller who manages this team or is a system admin (team-authority
    // fixes D3): they are who reaches this team's material from beneath it.
    // On the root or the gating team only a system admin: every personal team hangs under the root.
    let admin = crate::authz::Principal::from(caller)
        .system_admin(pool)
        .await?
        .is_some();
    let manages = admin
        || (!is_root_or_gating(pool, team_id).await?
            && matches!(
                role_on_team(pool, team_id, caller.profile_id()).await?,
                Some(role) if can_manage(role)
            ));
    let children = if manages {
        sqlx::query_as!(
            TeamChildRef,
            r#"SELECT t.id, t.slug, t.name
                 FROM kb_teams_parents tp
                 JOIN kb_teams t ON t.id = tp.child_id AND t.is_active
                WHERE tp.parent_id = $1
                ORDER BY t.slug"#,
            team_id,
        )
        .fetch_all(pool)
        .await?
    } else {
        Vec::new()
    };

    // The live parents, for a member of this team (any role) or a system admin (team DAG doors D9,
    // ruling R2 as amended): a member already reads their material, so the names disclose nothing
    // new. Not for a caller who reads this team only as an ancestor's manager: a child's other
    // parents may be teams that caller has no relationship with.
    let member = role_on_team(pool, team_id, caller.profile_id())
        .await?
        .is_some();
    let parents = if admin || member {
        let mut conn = pool.acquire().await?;
        live_parents(&mut conn, team_id).await?
    } else {
        Vec::new()
    };
    // public_within, as the team (T): the same audience as `parents`, never an ancestor's manager,
    // since an anchor on another branch is not theirs to see. As the anchor (A): the same audience
    // as `children`, its managers (on the root or the gating team, a system admin only). People
    // only, by the read arm's own fence (`public_within_reading`): a machine, a connection agent or
    // a profile with no human auth link never reads through a row, so it is not shown the rows
    // either.
    let person = (admin || member || manages)
        && matches!(caller, Caller::Human(_))
        && sqlx::query_scalar!(
            r#"SELECT public_within_reading($1) AS "person!""#,
            *caller.profile_id()
        )
        .fetch_one(pool)
        .await?;
    let public_within = if person && (admin || member) {
        public_within_of(pool, team_id).await?
    } else {
        Vec::new()
    };
    let public_within_in = if person && manages {
        public_within_in_of(pool, team_id).await?
    } else {
        Vec::new()
    };

    Ok(TeamDetail {
        id: team.id,
        slug: team.slug,
        name: team.name,
        description: team.description,
        created: team.created,
        auto_join_role: team.auto_join_role,
        members,
        children,
        parents,
        public_within,
        public_within_in,
    })
}

/// Update a team's mutable metadata (name/description). Owner/maintainer; on the root or the
/// gating team, a system admin only.
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
    // Auth before writes: owner or maintainer — and on the root or the gating team, a system admin
    // only (team-authority fixes D4).
    if is_root_or_gating(pool, team_id).await? {
        require_admin_on(pool, caller).await?;
    } else {
        match membership_row_role(pool, team_id, caller.profile_id()).await? {
            Some(role) if can_manage(role) => {}
            _ => return Err(ApiError::Forbidden),
        }
    }

    let mut tx = pool.begin().await?;
    // Under the team's row lock, the authority is decided again, as `rename_slug` does: a demotion
    // or role change committed since the pool check refuses the update.
    lock_teams_ascending(&mut tx, &[team_id]).await?;
    if crate::services::access_service::is_root_or_gating_team_on(&mut tx, team_id).await? {
        require_admin_in(&mut tx, caller).await?;
    } else if !matches!(
        membership_row_role(&mut *tx, team_id, caller.profile_id()).await?,
        Some(role) if can_manage(role)
    ) {
        return Err(ApiError::Forbidden);
    }
    let row = sqlx::query_as!(
        TeamRowDb,
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
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::NotFound("team not found or not readable".to_string()))?;
    tx.commit().await?;

    Ok(row.into())
}

/// Soft-delete a team (`is_active = false`). Owner, or a system admin; on the root or the gating
/// team, a system admin only; on a personal team, its owner only (task `01a19006`).
///
/// Refuses the `temper-system` root (load-bearing: every profile descends from
/// it), and the instance's gating team (`409`): the instance is never without a live gating team, so
/// the gating slug moves to another live team first. Idempotency: a team that is absent OR already soft-deleted yields
/// `NotFound`. Rows are preserved — [`restore_team`] is the inverse.
/// Children are NOT recursively deleted (see the migration's cascade note).
pub async fn delete_team(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
) -> ApiResult<PublicWithinEffects> {
    // Auth before writes: owner only (stricter than manage — a maintainer cannot dissolve the
    // team), or a system admin. On the root or the gating team, a system admin only
    // (team-authority fixes D4). The admin arm stops at a personal team, which only its owner and
    // erasure act on.
    let admin = personal_of(pool, team_id).await?.is_none()
        && crate::authz::Principal::Human(caller)
            .system_admin(pool)
            .await?
            .is_some();
    if !admin {
        if is_root_or_gating(pool, team_id).await? {
            return Err(ApiError::Forbidden);
        }
        match membership_row_role(pool, team_id, caller.profile_id()).await? {
            Some(TeamRole::Owner) => {}
            _ => return Err(ApiError::Forbidden),
        }
    }

    // The write and its `team_lifecycle_changed` record are one statement (lifecycle doors D8). The
    // writer guards the DAG root in its UPDATE's WHERE, so the check and the write stay atomic.
    let mut tx = pool.begin().await?;
    // Under the team's row lock, the authority is decided again: the admin arm on the locked
    // governance read, and otherwise the owner arm on the membership row, as above. A demotion or
    // removal committed since the pool check cannot race the delete it would have refused (nor be
    // told the rows the delete names to the team's owner); a demotion still in flight waits for it.
    lock_teams_ascending(&mut tx, &[team_id]).await?;
    let admin = admin && caller_is_admin_on(&mut tx, caller).await?;
    if !admin
        && (crate::services::access_service::is_root_or_gating_team_on(&mut tx, team_id).await?
            || membership_row_role(&mut *tx, team_id, caller.profile_id()).await?
                != Some(TeamRole::Owner))
    {
        return Err(ApiError::Forbidden);
    }
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller.profile_id(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (outcome, effects) = lifecycle_write(
        &mut tx,
        emitter.uuid(),
        caller.profile_id(),
        team_id,
        "deleted",
        None,
        None,
    )
    .await?;
    // Decided under the team's row lock and the shared gating lock, so a repoint committed since
    // any earlier read is seen. The instance is never without its gating team.
    // Only an admin reaches the gating team's delete on purpose; an owner whose team became the
    // gating team after the check above gets the refusal a non-admin gets there.
    if outcome == LifecycleOutcome::Gating {
        return Err(if admin {
            gating_team_cannot_be_deleted()
        } else {
            ApiError::Forbidden
        });
    }
    // The owner arm passed above, so the caller owned the team the delete just made dead.
    let owned = (!admin).then_some(team_id);
    let public_within = name_effects_owning(&mut tx, caller, owned, effects).await?;
    tx.commit().await?;

    if outcome == LifecycleOutcome::Absent {
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
    Ok(public_within)
}

/// The refusal of a delete aimed at the instance's gating team: it is named, and says how to proceed.
fn gating_team_cannot_be_deleted() -> ApiError {
    ApiError::Conflict(
        "the gating team cannot be deleted; point the gating slug at another live team first"
            .to_string(),
    )
}

/// Change a team's slug (team DAG doors D7) — its own door, not a `PATCH` field (review L3).
///
/// Authority as [`update_team`]: an owner or maintainer; on the root or the gating team, a system
/// admin only. Refused (`400`) for `temper-system` (its slug is load-bearing in SQL), a personal
/// team (`personal-<handle>` is derived identity), and a slug failing [`validate_team_slug`].
/// `409` on a collision with any slug, live or soft-deleted (deleted slugs stay reserved).
///
/// The gating team is stored by slug (`kb_system_settings.gating_team_slug`); the writer moves that
/// setting in the same statement under `_gating_team_lock`, so the instance is never without its
/// gating team. Addresses change: `+old/<ctx>` stops resolving; UUID refs are unaffected; refs in
/// stored content are not rewritten. Recorded as `team_lifecycle_changed` `renamed`.
pub async fn rename_slug(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    req: &RenameTeamSlugRequest,
) -> ApiResult<RenameTeamSlugOutcome> {
    if is_root_or_gating(pool, team_id).await? {
        require_admin_on(pool, caller).await?;
    } else {
        match membership_row_role(pool, team_id, caller.profile_id()).await? {
            Some(role) if can_manage(role) => {}
            _ => return Err(ApiError::Forbidden),
        }
    }
    let current = sqlx::query!(
        r#"SELECT slug, personal_of IS NOT NULL AS "personal!" FROM kb_teams WHERE id = $1"#,
        team_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("team not found or not readable".to_string()))?;
    if current.slug == "temper-system" {
        return Err(ApiError::BadRequest(
            "the temper-system root keeps its slug".to_string(),
        ));
    }
    if current.personal {
        return Err(ApiError::BadRequest(
            "a personal team's slug is derived from its owner".to_string(),
        ));
    }
    let to = validate_team_slug(&req.slug)?;

    let mut tx = pool.begin().await?;
    // Under the team's row lock, the authority is decided again, and the slug being replaced is
    // read: a concurrent demotion cannot race the rename, and the outcome names the slug that
    // actually stopped resolving.
    lock_teams_ascending(&mut tx, &[team_id]).await?;
    if crate::services::access_service::is_root_or_gating_team_on(&mut tx, team_id).await? {
        require_admin_in(&mut tx, caller).await?;
    } else if !role_on_team_on(&mut tx, team_id, caller.profile_id())
        .await?
        .is_some_and(can_manage)
    {
        return Err(ApiError::Forbidden);
    }
    let from = sqlx::query_scalar!("SELECT slug FROM kb_teams WHERE id = $1", team_id)
        .fetch_one(&mut *tx)
        .await?;
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller.profile_id(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (outcome, _rename_flips_no_row) = lifecycle_write(
        &mut tx,
        emitter.uuid(),
        caller.profile_id(),
        team_id,
        "renamed",
        Some(&to),
        None,
    )
    .await
    .map_err(|e| match e {
        // `kb_teams_slug_key`: any slug, live or soft-deleted.
        ApiError::Conflict(_) => ApiError::Conflict("team slug already exists".to_string()),
        other => other,
    })?;
    if outcome == LifecycleOutcome::Absent {
        return Err(ApiError::NotFound(
            "team not found or not readable".to_string(),
        ));
    }
    tx.commit().await?;
    Ok(RenameTeamSlugOutcome { team_id, from, to })
}

/// Restore a soft-deleted team (team DAG doors D6) — the inverse of [`delete_team`], with its
/// authority: the team's `owner` (the blind row lookup, since the team is dead), or a system admin;
/// on the root or the gating team, a system admin only; on a personal team, its owner only. Not a
/// `maintainer`: delete is owner-only.
///
/// `is_active = true` and its `team_lifecycle_changed` record are one statement. The slug is still
/// held (soft-deleted slugs stay reserved), so nothing collides. Every preserved row confers again:
/// the roster, the parent links (and through them the children's reach to everything above), its
/// contexts, shares and grants. So the outcome lists the live parents, the live children, the
/// roster and the full live ancestor chain: the restorer sees the reach they revived. After commit,
/// an auto-join team backfills the profiles approved while it was dead. `NotFound` when the team is absent or not deleted.
pub async fn restore_team(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
) -> ApiResult<RestoreTeamOutcome> {
    let admin = personal_of(pool, team_id).await?.is_none()
        && crate::authz::Principal::Human(caller)
            .system_admin(pool)
            .await?
            .is_some();
    if !admin {
        restore_authority(pool.acquire().await?.as_mut(), caller, team_id).await?;
    }

    let mut tx = pool.begin().await?;
    // Under the team's row lock, the authority is decided again: a concurrent demotion or removal
    // cannot race the restore it would have refused. An admin demoted since the pool check is
    // refused here rather than re-decided on the owner arm, which needs the subtree's locks taken
    // in one ascending pass; a retry decides on that arm.
    if admin {
        lock_teams_ascending(&mut tx, &[team_id]).await?;
        require_admin_in(&mut tx, caller).await?;
    } else {
        let stable = lock_teams_and_subtree(&mut tx, &[], team_id).await?;
        restore_authority(&mut tx, caller, team_id).await?;
        if !stable {
            return Err(subtree_changed());
        }
    }
    // Restore flips `is_active` only, and the pool walk counts dead teams, so this answers none
    // today; the admin arm runs the one scan anyway, so the doors cannot drift apart. The owner arm
    // never restores a pool's ancestor (`restore_authority`), so it does not scan.
    let elevation = if admin {
        Some(saml_mapping_service::elevation_before(&mut tx, &[team_id]).await?)
    } else {
        None
    };
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller.profile_id(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (outcome, effects) = lifecycle_write(
        &mut tx,
        emitter.uuid(),
        caller.profile_id(),
        team_id,
        "restored",
        None,
        None,
    )
    .await?;
    if outcome == LifecycleOutcome::Absent {
        return Err(ApiError::NotFound(
            "team not found or not deleted".to_string(),
        ));
    }
    let public_within = name_effects(&mut tx, caller, effects).await?;
    let ancestors = live_ancestors(&mut tx, team_id).await?;
    let elevated_mappings = saml_mapping_service::record_elevations(
        &mut tx,
        elevation,
        emitter.uuid(),
        caller.profile_id(),
        None,
    )
    .await?;
    tx.commit().await?;

    let team = sqlx::query_as!(
        TeamRowDb,
        r#"SELECT id, slug, name, description, created,
                  auto_join_role AS "auto_join_role: TeamRole"
             FROM kb_teams WHERE id = $1"#,
        team_id,
    )
    .fetch_one(pool)
    .await?;
    if team.auto_join_role.is_some() {
        sqlx::query!("SELECT backfill_auto_join_team($1)", team_id)
            .execute(pool)
            .await?;
    }
    let mut conn = pool.acquire().await?;
    Ok(RestoreTeamOutcome {
        parents: live_parents(&mut conn, team_id).await?,
        children: live_children(&mut conn, team_id).await?,
        members: roster(&mut conn, team_id).await?,
        team: team.into(),
        public_within,
        ancestors,
        elevated_mappings,
    })
}

/// The non-admin arm of [`restore_team`]: the team's `owner` on its membership row (the team is
/// dead, so the blind lookup), never the root or the gating team, and never an auto-join team —
/// restoring one re-enrols every approved profile, which is a system admin's act. The
/// same holds when such a team sits anywhere in the restored team's subtree, live or dead, and when
/// the team keeps a link to the root or the gating team, live or dead (a personal team excepted).
async fn restore_authority(
    conn: &mut sqlx::PgConnection,
    caller: &HumanPrincipal,
    team_id: Uuid,
) -> ApiResult<()> {
    // The team itself (root, gating or auto-join) or any team below it: restoring re-links that
    // subtree's reach to everything above the team.
    if subtree_holds_pool_on(conn, team_id).await? {
        return Err(ApiError::Forbidden);
    }
    // A preserved link to the root or the gating team, live or dead: restoring it re-links the team
    // under an instance team (at once, or when an admin restores that parent), which only a system
    // admin does (attach's rule for an instance parent). A personal team's only parent is the root,
    // and its owner restores it.
    let instance_parent = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1
               FROM kb_teams_parents tp
               JOIN kb_teams p ON p.id = tp.parent_id
               JOIN kb_teams c ON c.id = tp.child_id AND c.personal_of IS NULL
              WHERE tp.child_id = $1
                AND (p.slug = $2
                     OR p.slug = (SELECT ss.gating_team_slug FROM kb_system_settings ss
                                   WHERE ss.id = 1))) AS "e!: bool""#,
        team_id,
        crate::services::access_service::ROOT_TEAM_SLUG,
    )
    .fetch_one(&mut *conn)
    .await?;
    if instance_parent {
        return Err(ApiError::Forbidden);
    }
    let row = sqlx::query!(
        r#"SELECT tm.role AS "role: TeamRole"
             FROM kb_team_members tm
            WHERE tm.team_id = $1 AND tm.profile_id = $2"#,
        team_id,
        *caller.profile_id(),
    )
    .fetch_optional(&mut *conn)
    .await?;
    match row {
        // An auto-join team itself was refused above, with the rest of its subtree.
        Some(r) if r.role == TeamRole::Owner => Ok(()),
        _ => Err(ApiError::Forbidden),
    }
}

/// `team_id`'s live ancestors, itself excluded, ordered by slug: the reach an act that links,
/// moves, restores or pools the team confers on everyone who reads through it. The one reader for
/// every such answer (attach, reparent, restore, auto-join, and a settings write that repoints the
/// gating team), so the doors answer alike. It is
/// `team_ancestors`, the reach walk: live teams only, and the walk stops at a soft-deleted team, so
/// a team reached only through a dead one is not in reach and not named. Call it on the act's own
/// transaction, after the write, so the answer is the act's own view of what it wrote; ancestors
/// above the locked teams are not locked, so a concurrent change above them can still land before
/// commit. Empty for a dead team.
///
/// This names more than `team_detail` shows a member, which is the direct `parents` only: a
/// grandparent, or a team above the team's other parents, is named here even when it holds no
/// material. That is deliberate. The reach an act confers is shown to the caller who confers it,
/// so they can see what they opened. The caller is a system admin, or holds a live row on the team
/// itself (attach and reparent re-check managing it under the lock; restore requires its owner),
/// so every team named is one the caller's own team now reads through.
pub(crate) async fn live_ancestors(
    conn: &mut sqlx::PgConnection,
    team_id: Uuid,
) -> ApiResult<Vec<TeamRef>> {
    Ok(sqlx::query_as!(
        TeamRef,
        r#"SELECT t.id, t.slug, t.name
             FROM team_ancestors($1) a
             JOIN kb_teams t ON t.id = a.team_id
            WHERE a.team_id <> $1
            ORDER BY t.slug"#,
        team_id,
    )
    .fetch_all(&mut *conn)
    .await?)
}

async fn live_parents(conn: &mut sqlx::PgConnection, team_id: Uuid) -> ApiResult<Vec<TeamRef>> {
    Ok(sqlx::query_as!(
        TeamRef,
        r#"SELECT t.id, t.slug, t.name
             FROM kb_teams_parents tp
             JOIN kb_teams t ON t.id = tp.parent_id AND t.is_active
            WHERE tp.child_id = $1
            ORDER BY t.slug"#,
        team_id,
    )
    .fetch_all(&mut *conn)
    .await?)
}

async fn live_children(conn: &mut sqlx::PgConnection, team_id: Uuid) -> ApiResult<Vec<TeamRef>> {
    Ok(sqlx::query_as!(
        TeamRef,
        r#"SELECT t.id, t.slug, t.name
             FROM kb_teams_parents tp
             JOIN kb_teams t ON t.id = tp.child_id AND t.is_active
            WHERE tp.parent_id = $1
            ORDER BY t.slug"#,
        team_id,
    )
    .fetch_all(&mut *conn)
    .await?)
}

async fn roster(conn: &mut sqlx::PgConnection, team_id: Uuid) -> ApiResult<Vec<TeamMemberDetail>> {
    Ok(sqlx::query_as!(
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
    .fetch_all(&mut *conn)
    .await?)
}

/// Set, change or clear a team's `auto_join_role` (team DAG doors D5). A system admin's act.
///
/// Refused (`400`) on `temper-system` (the root already reaches everyone through personal teams),
/// the gating team (its roster is standing) and a personal team (it admits only its owner and their
/// machines), and for a role above `member` (a role that governs is granted by a person). The
/// column and its `team_lifecycle_changed` record are one statement; a change to the value already
/// held records nothing.
///
/// Ruling R3: existing rows are never touched. Auto-join rows are written `source = native` and are
/// not recorded, so nothing tells a pool-written row from one a manager wrote; a rewrite would
/// demote or evict people someone added deliberately. Setting or changing the role then backfills
/// (after commit) every eligible profile with no row; clearing it stops enrolment. The outcome
/// carries the team's live ancestors: with auto-join set, every approved profile reads them.
pub async fn set_auto_join(
    pool: &PgPool,
    admin: &SystemAdmin,
    team_id: Uuid,
    role: Option<TeamRole>,
) -> ApiResult<SetAutoJoinOutcome> {
    if role.is_some_and(|r| r.rank() > TeamRole::Member.rank()) {
        return Err(ApiError::BadRequest(
            "auto_join_role may be member or watcher; a role that governs a team is granted by a \
             person"
                .to_string(),
        ));
    }
    if is_root_or_gating(pool, team_id).await? {
        return Err(ApiError::BadRequest(
            "temper-system and the gating team are never auto-join teams".to_string(),
        ));
    }
    if personal_of(pool, team_id).await?.is_some() {
        return Err(ApiError::BadRequest(
            PERSONAL_TEAM_HOLDS_ITS_OWNER.to_string(),
        ));
    }

    let mut tx = pool.begin().await?;
    // `admin` was minted on the pool; under the team's row lock the standing is asked again, before
    // the write, so a demotion committed since refuses and one in flight waits for this act.
    lock_teams_ascending(&mut tx, &[team_id]).await?;
    if !crate::auth::is_system_admin_locked(&mut tx, admin.actor()).await? {
        return Err(ApiError::Forbidden);
    }
    let elevation = Some(saml_mapping_service::elevation_before(&mut tx, &[team_id]).await?);
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, admin.actor(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let wire = role.map(|r| r.as_str());
    let (outcome, effects) = lifecycle_write(
        &mut tx,
        emitter.uuid(),
        admin.actor(),
        team_id,
        "auto_join_changed",
        wire,
        None,
    )
    .await?;
    if outcome == LifecycleOutcome::Absent {
        return Err(ApiError::NotFound(
            "team not found or not readable".to_string(),
        ));
    }
    // The caller is a system admin, so every row is named.
    let mut public_within = PublicWithinEffects::default();
    for row in effects.lapsed {
        public_within
            .lapsed
            .push(row_ref(&mut tx, row.team_id, row.anchor_id).await?);
    }
    for change in effects.changes {
        let row = row_ref(&mut tx, change.team_id, change.anchor_id).await?;
        if change.effective {
            public_within.revived.push(row);
        } else {
            public_within.dormant.push(row);
        }
    }
    let ancestors = live_ancestors(&mut tx, team_id).await?;
    let elevated_mappings = saml_mapping_service::record_elevations(
        &mut tx,
        elevation,
        emitter.uuid(),
        admin.actor(),
        None,
    )
    .await?;
    tx.commit().await?;

    if role.is_some() {
        sqlx::query!("SELECT backfill_auto_join_team($1)", team_id)
            .execute(pool)
            .await?;
    }
    let team = sqlx::query_as!(
        TeamRowDb,
        r#"SELECT id, slug, name, description, created,
                  auto_join_role AS "auto_join_role: TeamRole"
             FROM kb_teams WHERE id = $1"#,
        team_id,
    )
    .fetch_one(pool)
    .await?;
    Ok(SetAutoJoinOutcome {
        team: team.into(),
        ancestors,
        public_within,
        elevated_mappings,
    })
}

/// Detach `child_id` from its parent `parent_id` (team-authority fixes D3; team DAG doors D2).
///
/// Rank stays per team (team-role rank spec R7): a parent's managers have no say in a child's
/// roster. What they do have, as they would over a member added to the parent directly, is the
/// means to end the reach: a child's members reach everything attached to the parent and its
/// ancestors, and this cuts that. The child and its roster survive; only this link is cut.
///
/// Auth before writes, any one arm (ruling R1):
/// - an `owner` or `maintainer` of the PARENT (direct, live), when the parent is neither the root
///   nor the gating team;
/// - an `owner` or `maintainer` of the CHILD (direct, live), from any parent, the gating team
///   included: detaching only removes reach the child's roster had, the team-level analogue of
///   "anyone may leave";
/// - a system admin.
///
/// The personal-team link under `temper-system` is structural and nobody removes it, its owner and
/// system admins included (task `01a19006`). A caller on no arm learns nothing about the link
/// (`Forbidden`); a caller on an arm asking to cut a link that does not exist gets `NotFound`. The
/// arms run again under both rows' locks, taken in ascending id order. The row and its
/// `team_parent_link_changed` record are one write.
pub async fn detach_child(
    pool: &PgPool,
    caller: &HumanPrincipal,
    parent_id: Uuid,
    child_id: Uuid,
) -> ApiResult<PublicWithinEffects> {
    if personal_of(pool, child_id).await?.is_some() {
        return Err(ApiError::Forbidden);
    }
    let admin = crate::authz::Principal::Human(caller)
        .system_admin(pool)
        .await?
        .is_some();
    if !admin {
        let mut conn = pool.acquire().await?;
        detach_authority(&mut conn, caller, parent_id, child_id).await?;
    }

    let mut tx = pool.begin().await?;
    // Under both rows' locks (the parent's is the one every membership writer takes), re-run the
    // arms, so a concurrent demotion or removal cannot race the detach it would have refused.
    lock_teams_ascending(&mut tx, &[parent_id, child_id]).await?;
    let admin = admin && caller_is_admin_on(&mut tx, caller).await?;
    if !admin {
        detach_authority(&mut tx, caller, parent_id, child_id).await?;
    }
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller.profile_id(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (outcome, effects) = unlink(
        &mut tx,
        emitter.uuid(),
        caller.profile_id(),
        parent_id,
        child_id,
        None,
    )
    .await?;
    if outcome != "unlinked" {
        return Err(ApiError::NotFound(
            "that team is not a child of this team".to_string(),
        ));
    }
    let public_within = name_effects(&mut tx, caller, effects).await?;
    tx.commit().await?;
    Ok(public_within)
}

/// The non-admin detach arms (D2): manages the live parent (not the root or the gating team), or
/// manages the live child. `Forbidden` on neither.
async fn detach_authority(
    conn: &mut sqlx::PgConnection,
    caller: &HumanPrincipal,
    parent_id: Uuid,
    child_id: Uuid,
) -> ApiResult<()> {
    let manages = |role: Option<TeamRole>| role.is_some_and(can_manage);
    // The child arm stops at the root and the gating team: their rosters are admin-only, and a
    // leftover managing row on either (a demoted admin's) confers nothing.
    if manages(role_on_team_on(conn, child_id, caller.profile_id()).await?)
        && !crate::services::access_service::is_root_or_gating_team_on(conn, child_id).await?
    {
        return Ok(());
    }
    if manages(role_on_team_on(conn, parent_id, caller.profile_id()).await?)
        && !crate::services::access_service::is_root_or_gating_team_on(conn, parent_id).await?
    {
        return Ok(());
    }
    Err(ApiError::Forbidden)
}

/// `_admin_team_parent_link_act(…, 'unlink', …)`: `unlinked` or `absent`, and the unfiltered
/// `public_within` effects of the cut (a row whose anchor is no longer above its team is deleted).
async fn unlink(
    tx: &mut sqlx::PgConnection,
    emitter: Uuid,
    actor: ProfileId,
    parent_id: Uuid,
    child_id: Uuid,
    correlation: Option<Uuid>,
) -> ApiResult<(String, ActEffects)> {
    let raw = sqlx::query_scalar!(
        r#"SELECT _admin_team_parent_link_act($1, $2, 'unlink', $3, $4, $5) AS "w!""#,
        emitter,
        *actor,
        parent_id,
        child_id,
        correlation,
    )
    .fetch_one(&mut *tx)
    .await?;
    link_answer(raw)
}

/// `_admin_team_parent_link_act(…, 'link', …)`: `linked` or `exists`, and the unfiltered
/// `public_within` effects, with the triggers' refusals mapped by [`map_link_trigger`].
async fn link(
    tx: &mut sqlx::PgConnection,
    emitter: Uuid,
    actor: ProfileId,
    parent_id: Uuid,
    child_id: Uuid,
    correlation: Option<Uuid>,
) -> ApiResult<(String, ActEffects)> {
    let raw = sqlx::query_scalar!(
        r#"SELECT _admin_team_parent_link_act($1, $2, 'link', $3, $4, $5) AS "w!""#,
        emitter,
        *actor,
        parent_id,
        child_id,
        correlation,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(map_link_trigger)?;
    link_answer(raw)
}

fn link_answer(raw: serde_json::Value) -> ApiResult<(String, ActEffects)> {
    let write: ActWrite = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("_admin_team_parent_link_act: {e}")))?;
    Ok((write.outcome, write.effects))
}

/// Resolve a team ref — `+slug`, a bare slug, a team UUID, or a decorated `slug-<uuid>` ref — to a
/// team id, live or not. `None` when nothing matches. Not an authority answer.
async fn resolve_team_ref(pool: &PgPool, team_ref: &str) -> ApiResult<Option<Uuid>> {
    let token = team_slug(team_ref);
    if let Ok(id) = temper_core::refs::parse_ref(token) {
        let id = Uuid::from(id);
        let exists = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM kb_teams WHERE id = $1) AS "e!""#,
            id
        )
        .fetch_one(pool)
        .await?;
        return Ok(exists.then_some(id));
    }
    Ok(
        sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = $1", token)
            .fetch_optional(pool)
            .await?,
    )
}

async fn team_ref_of(conn: &mut sqlx::PgConnection, team_id: Uuid) -> ApiResult<TeamRef> {
    Ok(sqlx::query_as!(
        TeamRef,
        "SELECT id, slug, name FROM kb_teams WHERE id = $1",
        team_id
    )
    .fetch_one(&mut *conn)
    .await?)
}

/// Move `child_id` from parent `from` to parent `to` in one act (team DAG doors D3).
///
/// Authority: detach from `from` per D2 and attach to `to` per D1. Attach already requires managing
/// the child, which is a D2 arm, so a caller who may attach may also leave: the caller manages the
/// child and `to` (or is a system admin), and a root or gating `to`, or a child whose subtree (live
/// or dead) holds an auto-join or gating team, is a system admin's act. A caller who does not manage the
/// child learns nothing (`403`, before refs are resolved). `from == to` is `400`; `from` that is
/// not a current parent is `404`.
///
/// One transaction: `from`, `to` and the child (and, for a non-admin, the child's subtree) are
/// locked in ascending id order and the checks run again; then `link` and only then `unlink`, so a
/// cycle refusal (`409`) aborts before anything is cut. Between two separate calls the child would
/// hold both reaches, and a failure would leave it there; here it never does. Both `team_parent_link_changed` records share one correlation id.
pub async fn reparent(
    pool: &PgPool,
    caller: &HumanPrincipal,
    child_id: Uuid,
    req: &ReparentRequest,
) -> ApiResult<ReparentOutcome> {
    let admin = crate::authz::Principal::Human(caller)
        .system_admin(pool)
        .await?
        .is_some();
    if !admin
        && !membership_row_role(pool, child_id, caller.profile_id())
            .await?
            .is_some_and(can_manage)
    {
        return Err(ApiError::Forbidden);
    }
    let not_found = |what: &str| ApiError::NotFound(format!("{what} team not found"));
    let from = resolve_team_ref(pool, &req.from)
        .await?
        .ok_or_else(|| not_found("`from`"))?;
    // An unknown `to` is answered as one the caller does not manage (403), so the door is not an
    // oracle for which team slugs exist (G6); only a system admin is told it is absent.
    let to = match resolve_team_ref(pool, &req.to).await? {
        Some(id) => id,
        None if admin => return Err(not_found("`to`")),
        None => return Err(ApiError::Forbidden),
    };
    if from == to {
        return Err(ApiError::BadRequest(
            "`from` and `to` name the same team".to_string(),
        ));
    }
    attach_precheck(pool, caller, admin, to, child_id).await?;

    let mut tx = pool.begin().await?;
    let stable = if admin {
        lock_teams_ascending(&mut tx, &[from, to, child_id]).await?;
        true
    } else {
        lock_teams_and_subtree(&mut tx, &[from, to], child_id).await?
    };
    for team in [to, child_id] {
        if !link_side(&mut tx, team)
            .await?
            .is_some_and(|side| side.live)
        {
            return Err(ApiError::NotFound(
                "team not found or not readable".to_string(),
            ));
        }
    }
    attach_authority(&mut tx, caller, admin, to, child_id).await?;
    if !stable {
        return Err(subtree_changed());
    }

    let correlation = Uuid::now_v7();
    // A pool moved under `to` is a system admin's act (`attach_authority`), so only that arm scans.
    let elevation = if admin {
        Some(saml_mapping_service::elevation_before(&mut tx, &[to]).await?)
    } else {
        None
    };
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller.profile_id(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (_, mut effects) = link(
        &mut tx,
        emitter.uuid(),
        caller.profile_id(),
        to,
        child_id,
        Some(correlation),
    )
    .await?;
    let (cut, cut_effects) = unlink(
        &mut tx,
        emitter.uuid(),
        caller.profile_id(),
        from,
        child_id,
        Some(correlation),
    )
    .await?;
    if cut != "unlinked" {
        // Dropping the transaction rolls the new link back with it.
        return Err(ApiError::NotFound(
            "`from` is not a parent of this team".to_string(),
        ));
    }
    effects.absorb(cut_effects);
    let public_within = name_effects(&mut tx, caller, effects).await?;
    let outcome = ReparentOutcome {
        team: team_ref_of(&mut tx, child_id).await?,
        from: team_ref_of(&mut tx, from).await?,
        to: team_ref_of(&mut tx, to).await?,
        correlation_id: correlation,
        public_within,
        ancestors: live_ancestors(&mut tx, child_id).await?,
        elevated_mappings: saml_mapping_service::record_elevations(
            &mut tx,
            elevation,
            emitter.uuid(),
            caller.profile_id(),
            Some(correlation),
        )
        .await?,
    };
    tx.commit().await?;
    Ok(outcome)
}

/// A team's standing for the attach door: live, personal, the root. Whether it (or anything below
/// it) is an instance pool is `subtree_holds_pool_on`'s question.
struct LinkSide {
    live: bool,
    personal: bool,
    root: bool,
}

async fn link_side(conn: &mut sqlx::PgConnection, team_id: Uuid) -> ApiResult<Option<LinkSide>> {
    let row = sqlx::query!(
        r#"SELECT is_active AS "live!",
                  personal_of IS NOT NULL AS "personal!",
                  slug = 'temper-system' AS "root!"
             FROM kb_teams WHERE id = $1"#,
        team_id,
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|r| LinkSide {
        live: r.live,
        personal: r.personal,
        root: r.root,
    }))
}

/// The instance-pool rule over the whole subtree: is `team_id` itself, or any team below it, live
/// or dead, an auto-join team, the root or the gating team? Any one of them makes every approved
/// profile a member of the subtree, so linking the subtree under a team (or restoring its top)
/// hands that team's material to the whole instance.
///
/// The walk goes through dead teams and counts them. A pool below a dead team reaches nothing
/// today, but it is latent reach: an admin restoring the dead team (or the pool) hands the parent,
/// and everything above it, to the instance, and a non-admin must not be able to set that up. This
/// is an authority walk, not reach, so it differs from `team_live_descendants` on purpose; the
/// subtree it reads is the one `lock_teams_and_subtree` locks, dead teams included.
///
/// The walk is `_team_subtree_holds_pool` (migration `20261126000010`), which reads
/// `_teams_over_pool`: the one definition of the predicate, shared with the SAML mapping door's
/// confirmation rule and its elevation scan, so the three cannot disagree. Its root and gating test
/// is `access_service::is_root_or_gating_team_on`'s: the `temper-system` slug and the same
/// `kb_system_settings` read. Change them together.
async fn subtree_holds_pool_on(conn: &mut sqlx::PgConnection, team_id: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT _team_subtree_holds_pool($1) AS "e!: bool""#,
        team_id
    )
    .fetch_one(&mut *conn)
    .await?)
}

/// Map a refusal from the `kb_teams_parents` triggers to its status: the cycle check is a `409`,
/// a personal-team link a `400`, the root taking a parent a `400`. Matched on the trigger's message
/// (`team_parent_link_refuses_cycle`, `personal_team_link_refused`), the only thing that tells two
/// `check_violation`s apart. Anything else passes through.
fn map_link_trigger(err: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(db) = &err {
        if db.code().as_deref() == Some("23514") {
            let msg = db.message();
            if msg.contains("one of its own descendants") {
                return ApiError::Conflict(
                    "that link would make a team its own ancestor".to_string(),
                );
            }
            if msg.contains("personal team") {
                return ApiError::BadRequest(PERSONAL_TEAM_HOLDS_ITS_OWNER.to_string());
            }
            if msg.contains("temper-system is the root") {
                return ApiError::BadRequest(
                    "temper-system is the root of the team tree and takes no parent".to_string(),
                );
            }
        }
    }
    ApiError::from(err)
}

/// The non-structural attach rules (team DAG doors D1, rules 1–4), on `conn`: the caller manages
/// the parent and the child (direct, live), unless a system admin; a root or gating parent, or a
/// child whose subtree (live or dead) holds an auto-join or gating team, is a system admin's act
/// alone. Run under the row locks, after [`attach_precheck`] decided the same on the pool.
async fn attach_authority(
    conn: &mut sqlx::PgConnection,
    caller: &HumanPrincipal,
    admin: bool,
    parent_id: Uuid,
    child_id: Uuid,
) -> ApiResult<()> {
    if admin {
        // The admin arm, asked again under the locks: a demotion since the pre-check refuses. It
        // is not re-decided on the managing arms, which need the child's subtree locked in the
        // same ascending pass; a retry decides on those.
        return require_admin_in(conn, caller).await;
    }
    for team in [parent_id, child_id] {
        match role_on_team_on(conn, team, caller.profile_id()).await? {
            Some(role) if can_manage(role) => {}
            _ => return Err(ApiError::Forbidden),
        }
    }
    let instance_parent =
        crate::services::access_service::is_root_or_gating_team_on(conn, parent_id).await?;
    // An auto-join team or an instance team (the gating team's roster is the access pool,
    // every approved profile) anywhere in the child's subtree, live or dead, hands (or, once
    // restored, would hand) the parent's material to
    // the whole instance: the same act as attaching to the root. The child itself is only the
    // first such team, not the only one. A leftover managing row on the gating team confers
    // nothing.
    let pool_child = subtree_holds_pool_on(conn, child_id).await?;
    if instance_parent || pool_child {
        // Re-asked, not reused: under the locks a demotion since the pre-check must refuse. On the
        // transaction's own connection: it holds rows, so it never waits on a second one.
        require_admin_in(conn, caller).await?;
    }
    Ok(())
}

/// The pool pre-check of [`attach_child`] (D1), in its order: rules 1–2 on the membership rows,
/// 3–4, then the structural refusals. Shared with [`reparent`]'s attach half.
async fn attach_precheck(
    pool: &PgPool,
    caller: &HumanPrincipal,
    admin: bool,
    parent_id: Uuid,
    child_id: Uuid,
) -> ApiResult<()> {
    // 1–2 on the membership rows, live or not: a dead team is told apart (404) only to a caller
    // who holds a managing row on both sides, after which 1–2 run again live-only under the locks.
    if !admin {
        for team in [parent_id, child_id] {
            match membership_row_role(pool, team, caller.profile_id()).await? {
                Some(role) if can_manage(role) => {}
                _ => return Err(ApiError::Forbidden),
            }
        }
    }
    let not_found = || ApiError::NotFound("team not found or not readable".to_string());
    let mut conn = pool.acquire().await?;
    let parent = link_side(&mut conn, parent_id)
        .await?
        .ok_or_else(not_found)?;
    let child = link_side(&mut conn, child_id)
        .await?
        .ok_or_else(not_found)?;
    // 3–4, still before any structural answer.
    let instance_parent =
        crate::services::access_service::is_root_or_gating_team_on(&mut conn, parent_id).await?;
    // The child itself, or any team below it, live or dead: see `attach_authority`.
    let pool_child = subtree_holds_pool_on(&mut conn, child_id).await?;
    if (instance_parent || pool_child) && !admin {
        return Err(ApiError::Forbidden);
    }
    // 5.
    if !parent.live || !child.live {
        return Err(not_found());
    }
    if parent.personal {
        return Err(ApiError::BadRequest(
            "a personal team cannot have child teams".to_string(),
        ));
    }
    if child.personal {
        return Err(ApiError::BadRequest(
            "a personal team's only parent is temper-system".to_string(),
        ));
    }
    if child.root {
        return Err(ApiError::BadRequest(
            "temper-system is the root of the team tree and takes no parent".to_string(),
        ));
    }
    if parent_id == child_id {
        return Err(ApiError::BadRequest(
            "a team cannot be its own parent".to_string(),
        ));
    }
    Ok(())
}

/// Attach `child_id` under `parent_id` (team DAG doors D1) — the mirror of [`detach_child`], and the
/// door for a second parent. A parent link is read reach: every member of the child, at any role,
/// reaches everything attached to the parent and every live ancestor. So writing one is conferring
/// read on a roster, and both sides consent.
///
/// Authority, all required (a system admin satisfies 1–2, never 5):
/// 1. the caller manages the parent (direct, live `owner`/`maintainer`): the side whose material
///    becomes readable consents;
/// 2. the caller manages the child: otherwise a parent's manager could adopt any team, list it
///    under `children`, and learn it exists;
/// 3. a parent that is the root or the gating team → a system admin only;
/// 4. a child whose subtree holds an auto-join or gating team (the child itself, or any team below
///    it, live or dead) → a system admin only: that team's roster is every approved
///    profile, so this is the same act as attaching to the root;
/// 5. structural: a personal team on either side, the root as the child, or the team as its own
///    parent is `400`; a link that closes a cycle is `409`; a soft-deleted side is `404`.
///
/// The checks run in that order, so a caller who manages neither side learns nothing (`403`); a
/// `404` reaches only a caller who holds a managing row on both. Then, under both rows' locks taken
/// in ascending id order, 1–4 run again (the G12 pattern of `create_team` and `detach_child`). The
/// row and its `team_parent_link_changed` record are one write; an existing link answers `exists`
/// and records nothing.
pub async fn attach_child(
    pool: &PgPool,
    caller: &HumanPrincipal,
    parent_id: Uuid,
    child_id: Uuid,
) -> ApiResult<AttachChildOutcome> {
    let admin = crate::authz::Principal::Human(caller)
        .system_admin(pool)
        .await?
        .is_some();

    attach_precheck(pool, caller, admin, parent_id, child_id).await?;

    let mut tx = pool.begin().await?;
    let stable = if admin {
        lock_teams_ascending(&mut tx, &[parent_id, child_id]).await?;
        true
    } else {
        lock_teams_and_subtree(&mut tx, &[parent_id], child_id).await?
    };
    for team in [parent_id, child_id] {
        if !link_side(&mut tx, team)
            .await?
            .is_some_and(|side| side.live)
        {
            return Err(ApiError::NotFound(
                "team not found or not readable".to_string(),
            ));
        }
    }
    attach_authority(&mut tx, caller, admin, parent_id, child_id).await?;
    if !stable {
        return Err(subtree_changed());
    }

    // The rows the child's roster reads before the link, under the effects lock the link's own
    // transitions take, so a concurrent open cannot move the difference.
    effects_lock(&mut tx).await?;
    let before = rows_in_reach(&mut tx, child_id).await?;
    // The SAML mappings over a pool before the link, after the row locks: a pool brought under
    // the parent elevates every authoring mapping above it that had none.
    // A pool attached under the parent is a system admin's act (`attach_authority`), so only that
    // arm scans.
    let elevation = if admin {
        Some(saml_mapping_service::elevation_before(&mut tx, &[parent_id]).await?)
    } else {
        None
    };
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller.profile_id(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (outcome, effects) = link(
        &mut tx,
        emitter.uuid(),
        caller.profile_id(),
        parent_id,
        child_id,
        None,
    )
    .await?;
    let public_within = name_effects(&mut tx, caller, effects).await?;
    let after = rows_in_reach(&mut tx, child_id).await?;
    let public_within_in_reach = name_reach(&mut tx, caller, &before, after).await?;
    let ancestors = live_ancestors(&mut tx, child_id).await?;
    let elevated_mappings = saml_mapping_service::record_elevations(
        &mut tx,
        elevation,
        emitter.uuid(),
        caller.profile_id(),
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(AttachChildOutcome {
        outcome: if outcome == "linked" {
            AttachOutcome::Linked
        } else {
            AttachOutcome::Exists
        },
        public_within,
        public_within_in_reach,
        ancestors,
        elevated_mappings,
    })
}

/// Lock `teams`, `root` and every team below `root`, in one ascending id order, before a non-admin
/// act re-checks that subtree for an instance pool (`subtree_holds_pool_on`).
///
/// The invariant: the re-check reads a subtree no other transaction can change before this one
/// commits. Every act that can put a pool below `root` (an attach under a team below it, a restore
/// of a team below it) locks the row it links under or restores, and every one of those rows is in
/// this lock set, DEAD ONES INCLUDED, so the two acts share a row lock and the later one re-checks
/// against the earlier one's committed state. `root` is walked below whether or not it is live,
/// because a restore asks this while the team is still dead. The gating slug is then held, through the
/// shared side of `_gating_team_lock`, so the gating team cannot be repointed into the subtree
/// mid-act.
///
/// One ascending pass, never a second: a later pass would take rows out of order and could
/// deadlock against another door's ascending pass. So the subtree is walked, locked, and walked
/// again. Returns whether the second walk found nothing new. When it found a team linked in while
/// this act waited for the locks, the caller still runs its re-check (which reads committed state,
/// so it refuses a pool that arrived that way), and if the re-check passes, refuses with
/// [`subtree_changed`] rather than acting on a subtree it has not locked in full.
async fn lock_teams_and_subtree(
    tx: &mut sqlx::PgConnection,
    teams: &[Uuid],
    root: Uuid,
) -> ApiResult<bool> {
    let below = every_team_below(tx, root).await?;
    let mut all: Vec<Uuid> = teams
        .iter()
        .copied()
        .chain(std::iter::once(root))
        .chain(below.iter().copied())
        .collect();
    all.sort();
    all.dedup();
    // One statement, one round trip: the row lock sits above the sort, so the rows are locked in
    // ascending id order, the same order `lock_teams_ascending` takes them in.
    sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM kb_teams WHERE id = ANY($1) ORDER BY id FOR NO KEY UPDATE"#,
        &all,
    )
    .fetch_all(&mut *tx)
    .await?;
    // The gating slug, held for the re-check: the shared side of `_gating_team_lock`, which every
    // gating slug change takes exclusively (the settings trigger, and every rename). Taken AFTER
    // the team rows, in the order `rename_slug` takes them (its team row, then the exclusive
    // lock), so this act never holds the advisory lock while waiting on a row. It never touches
    // the settings row. (The gating-slug writer, `_admin_gating_team_slug_write`, takes the
    // exclusive lock before the settings row, the order a rename takes them in.)
    sqlx::query_scalar!(r#"SELECT 1 AS "one!" FROM (SELECT _gating_team_lock(false)) g"#)
        .fetch_one(&mut *tx)
        .await?;
    Ok(every_team_below(tx, root).await?.is_subset(&below))
}

/// The retryable refusal when the subtree grew while the act waited for its locks.
fn subtree_changed() -> ApiError {
    ApiError::Conflict("the team tree changed while this act was being checked; retry".to_string())
}

/// Every team below `root` through `kb_teams_parents`, live or dead: the lock set's walk, which must
/// not stop at a dead team (see `lock_teams_and_subtree`).
async fn every_team_below(
    tx: &mut sqlx::PgConnection,
    root: Uuid,
) -> ApiResult<std::collections::BTreeSet<Uuid>> {
    Ok(sqlx::query_scalar!(
        r#"WITH RECURSIVE down AS (
               SELECT tp.child_id AS team_id FROM kb_teams_parents tp WHERE tp.parent_id = $1
               UNION
               SELECT tp.child_id FROM kb_teams_parents tp JOIN down ON tp.parent_id = down.team_id
           )
           SELECT team_id AS "team_id!" FROM down"#,
        root,
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect())
}

/// Take `FOR NO KEY UPDATE` on each team row, in ascending id order, so two doors locking the same
/// pair cannot deadlock. A missing row takes no lock and is not an error here; the caller re-reads.
async fn lock_teams_ascending(tx: &mut sqlx::PgConnection, teams: &[Uuid]) -> ApiResult<()> {
    let mut ids = teams.to_vec();
    ids.sort();
    ids.dedup();
    for id in ids {
        sqlx::query!(
            "SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE",
            id
        )
        .fetch_optional(&mut *tx)
        .await?;
    }
    Ok(())
}

/// Remove a member from a team. A manager removes members up to their own rank who are not owners;
/// anyone may leave (team-role rank spec C2, C3, R6). Only a system admin removes another owner, or
/// anyone from the gating team or temper-system other than themselves (R11). Refuses an identity-provider row (C7),
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
/// - on the gating team and `temper-system`, only a system admin changes roles;
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

// =================================================================================================
// public_within: the doors (open, close, the forbid) and the discovery list
// =================================================================================================

/// The refusal for an open over a team some forbid covers (its own, one above it, or the root's,
/// which is the instance-wide off). Shown only to a caller who has already passed the side checks
/// that read it: the root's forbid is the same for every team, and a subtree forbid is answered
/// only to a manager of the team.
pub const PUBLIC_WITHIN_FORBIDDEN_HERE: &str =
    "public_within_forbidden_here: a forbid applies over this team (on it, on a team above it, or \
     on temper-system)";

/// The refusal to a manager of the anchor when the anchor is the root or the gating team, or its
/// subtree holds an auto-join or the gating team: only a system admin opens such a row. One message
/// for every case, so it does not say which pool team sits below.
pub const PUBLIC_WITHIN_ADMIN_ANCHOR: &str =
    "opening to this anchor requires a system admin: its subtree reaches the whole instance";

async fn caller_is_admin(pool: &PgPool, caller: &HumanPrincipal) -> ApiResult<bool> {
    Ok(crate::authz::Principal::Human(caller)
        .system_admin(pool)
        .await?
        .is_some())
}

/// [`caller_is_admin`] on the transaction's own connection, under its locks: a re-check never takes
/// a second pooled connection while the transaction holds rows, and a demotion serializes with it
/// (`auth::require_system_admin_locked`).
async fn caller_is_admin_on(
    conn: &mut sqlx::PgConnection,
    caller: &HumanPrincipal,
) -> ApiResult<bool> {
    match crate::auth::require_system_admin_locked(conn, caller).await {
        Ok(_) => Ok(true),
        Err(ApiError::Forbidden) => Ok(false),
        Err(other) => Err(other),
    }
}

/// A direct, live `owner` or `maintainer` row on the team.
async fn manages_on(
    conn: &mut sqlx::PgConnection,
    team_id: Uuid,
    caller: ProfileId,
) -> ApiResult<bool> {
    Ok(role_on_team_on(conn, team_id, caller)
        .await?
        .is_some_and(can_manage))
}

/// What an open decided, once its rules passed.
#[derive(Debug, Clone, Copy)]
struct OpenDecision {
    admin_act: bool,
    instance_wide: bool,
}

/// The open's rules, in order, on `conn` (the pool for the pre-check, the transaction under the
/// open's locks for the decision). `admin` is the caller's system-admin standing, read by the
/// caller of this function each time.
///
/// 1. The root forbid, the instance-wide off: the same for every team, so it says nothing about
///    this one.
/// 2. A direct, live manager of the team. No admin bypass: the team's consent is always given by
///    one of its own managers.
/// 3. A direct, live manager of the anchor, or a system admin.
/// 4. The anchor is the root or the gating team: a system admin only.
/// 5. The anchor's live subtree holds an auto-join or the gating team: a system admin only, and the
///    row is `instance_wide`.
/// 6. No forbid over the team, by any path, live or dead.
/// 7. The anchor is a live strict ancestor of the team.
/// 8. The team is not personal, the root or the gating team; the anchor is not personal.
///
/// A caller who manages neither side stops at rule 2 or 3 with the bare `403`, whatever the teams
/// are; a subtree forbid is named only after rule 2, to a manager of the team.
async fn open_rules(
    conn: &mut sqlx::PgConnection,
    caller: &HumanPrincipal,
    admin: bool,
    team_id: Uuid,
    anchor_id: Uuid,
) -> ApiResult<OpenDecision> {
    let me = caller.profile_id();
    let root_forbidden = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM kb_teams
                           WHERE slug = 'temper-system' AND public_within_forbidden) AS "f!""#
    )
    .fetch_one(&mut *conn)
    .await?;
    if root_forbidden {
        return Err(ApiError::ForbiddenDetail(
            PUBLIC_WITHIN_FORBIDDEN_HERE.to_string(),
        ));
    }
    if !manages_on(conn, team_id, me).await? {
        return Err(ApiError::Forbidden);
    }
    let manages_anchor = manages_on(conn, anchor_id, me).await?;
    if !manages_anchor && !admin {
        return Err(ApiError::Forbidden);
    }
    let shape = sqlx::query!(
        r#"SELECT team_is_root_or_gating($2)                     AS "anchor_root_or_gating!",
                  anchor_subtree_holds_pool($2)                  AS "pool!",
                  public_within_allowed_over($1)                 AS "allowed!",
                  ($1 <> $2 AND EXISTS (SELECT 1 FROM team_ancestors($1) a
                                         WHERE a.team_id = $2))  AS "ancestor!",
                  EXISTS (SELECT 1 FROM kb_teams t
                           WHERE t.id = $1 AND t.personal_of IS NOT NULL)
                    OR team_is_root_or_gating($1)
                    OR EXISTS (SELECT 1 FROM kb_teams t
                                WHERE t.id = $2 AND t.personal_of IS NOT NULL) AS "barred!""#,
        team_id,
        anchor_id,
    )
    .fetch_one(&mut *conn)
    .await?;
    let admin_required = shape.anchor_root_or_gating || shape.pool;
    if admin_required && !admin {
        return Err(ApiError::ForbiddenDetail(
            PUBLIC_WITHIN_ADMIN_ANCHOR.to_string(),
        ));
    }
    if !shape.allowed {
        return Err(ApiError::ForbiddenDetail(
            PUBLIC_WITHIN_FORBIDDEN_HERE.to_string(),
        ));
    }
    if !shape.ancestor {
        return Err(ApiError::BadRequest(
            "the anchor must be a live team above this one".to_string(),
        ));
    }
    if shape.barred {
        return Err(ApiError::BadRequest(
            "a personal team, temper-system or the gating team cannot be opened, and a personal \
             team cannot be an anchor"
                .to_string(),
        ));
    }
    Ok(OpenDecision {
        admin_act: admin && (admin_required || !manages_anchor),
        instance_wide: shape.pool,
    })
}

/// Open team `team_id`'s owned contexts to the people under `anchor_id` (`public_within(T, A)`).
///
/// Both sides consent: the caller manages the team directly, and manages the anchor directly or is
/// a system admin; the rules and their order are on `open_rules`. They run on the pool, then
/// again in the transaction after `_public_within_open_locks` has locked the team and the anchor
/// (and, shared, every team whose forbid reaches the team), so a demotion or a forbid committed
/// while the open waited is seen. If the team gained an ancestor while the open waited, the lock
/// set is stale and the open answers a retryable `409`. The row and its `team_public_within_changed` record are one
/// statement. `exists` when the row is already there: nothing is written.
pub async fn open_public_within(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    anchor_id: Uuid,
) -> ApiResult<PublicWithinOutcome> {
    {
        let admin = caller_is_admin(pool, caller).await?;
        let mut conn = pool.acquire().await?;
        open_rules(&mut conn, caller, admin, team_id, anchor_id).await?;
    }

    let mut tx = pool.begin().await?;
    let stable = sqlx::query_scalar!(
        r#"SELECT _public_within_open_locks($1, $2) AS "stable!""#,
        team_id,
        anchor_id,
    )
    .fetch_one(&mut *tx)
    .await?;
    if !stable {
        return Err(subtree_changed());
    }
    let admin = caller_is_admin_on(&mut tx, caller).await?;
    let decision = open_rules(&mut tx, caller, admin, team_id, anchor_id).await?;
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller.profile_id(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let written = sqlx::query_scalar!(
        r#"SELECT _admin_public_within_write($1, $2, $3, $4, 'opened', $5, $6, NULL) AS "o!""#,
        emitter.uuid(),
        *caller.profile_id(),
        team_id,
        anchor_id,
        decision.admin_act,
        decision.instance_wide,
    )
    .fetch_one(&mut *tx)
    .await?;
    let instance_wide = sqlx::query_scalar!(
        "SELECT instance_wide FROM kb_team_public_within WHERE team_id = $1 AND anchor_id = $2",
        team_id,
        anchor_id,
    )
    .fetch_one(&mut *tx)
    .await?;
    let audience = audience_on(&mut tx, anchor_id).await?;
    tx.commit().await?;
    Ok(PublicWithinOutcome {
        outcome: if written == "opened" {
            PublicWithinOpenKind::Opened
        } else {
            PublicWithinOpenKind::Exists
        },
        audience,
        instance_wide,
    })
}

async fn audience_on(
    conn: &mut sqlx::PgConnection,
    anchor_id: Uuid,
) -> ApiResult<PublicWithinAudience> {
    let row = sqlx::query!(
        r#"SELECT teams AS "teams!", principals AS "principals!"
             FROM public_within_audience($1)"#,
        anchor_id,
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(PublicWithinAudience {
        teams: row.teams,
        principals: row.principals,
    })
}

/// The close's arms: a direct, live manager of the team; a direct, live manager of the anchor
/// unless the anchor is the root or the gating team (a leftover owner row there confers nothing);
/// or a system admin. `Some(true)` when only the admin arm admits the caller.
async fn close_authority(
    conn: &mut sqlx::PgConnection,
    caller: &HumanPrincipal,
    admin: bool,
    team_id: Uuid,
    anchor_id: Uuid,
) -> ApiResult<bool> {
    let me = caller.profile_id();
    if manages_on(conn, team_id, me).await? {
        return Ok(false);
    }
    if manages_on(conn, anchor_id, me).await?
        && !crate::services::access_service::is_root_or_gating_team_on(conn, anchor_id).await?
    {
        return Ok(false);
    }
    if admin {
        return Ok(true);
    }
    Err(ApiError::Forbidden)
}

/// Close `public_within(team_id, anchor_id)`: delete the row and record it (`closed`). Either side
/// closes alone, whatever the policy or either team's liveness says: a direct, live manager of the
/// team, a direct, live manager of the anchor (on the root or the gating team, an admin only), or a
/// system admin. A caller on no arm gets the bare `403` whether or not the row or the teams exist;
/// a caller on an arm gets `404` when there is no such row. The arms run again under both team rows'
/// locks.
pub async fn close_public_within(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    anchor_id: Uuid,
) -> ApiResult<()> {
    {
        let admin = caller_is_admin(pool, caller).await?;
        let mut conn = pool.acquire().await?;
        close_authority(&mut conn, caller, admin, team_id, anchor_id).await?;
    }
    let mut tx = pool.begin().await?;
    lock_teams_ascending(&mut tx, &[team_id, anchor_id]).await?;
    // After the rows, as every holder takes it: an act computing transitions either sees this close
    // committed (and skips the row) or finishes before it, so no record names a row being closed.
    effects_lock(&mut tx).await?;
    let admin = caller_is_admin_on(&mut tx, caller).await?;
    let admin_act = close_authority(&mut tx, caller, admin, team_id, anchor_id).await?;
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller.profile_id(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let written = sqlx::query_scalar!(
        r#"SELECT _admin_public_within_write($1, $2, $3, $4, 'closed', $5, false, NULL) AS "o!""#,
        emitter.uuid(),
        *caller.profile_id(),
        team_id,
        anchor_id,
        admin_act,
    )
    .fetch_one(&mut *tx)
    .await?;
    if written != "closed" {
        return Err(ApiError::NotFound(
            "no public_within row for this team and anchor".to_string(),
        ));
    }
    tx.commit().await?;
    Ok(())
}

/// Who may set a team's forbid. One rule for every team: on the root (`temper-system`, where the
/// forbid is the instance-wide off) and on the gating team, a system admin only, and an owner row
/// there confers nothing; elsewhere the team's direct, live `owner`, or a system admin. A caller on
/// no arm gets the bare `403`. A personal team has no forbid (`400`), for its owner and an admin
/// alike; that is checked after the arms, so it is answered only to someone on one.
async fn policy_authority(
    conn: &mut sqlx::PgConnection,
    caller: &HumanPrincipal,
    admin: bool,
    team_id: Uuid,
) -> ApiResult<()> {
    let admitted = if crate::services::access_service::is_root_or_gating_team_on(conn, team_id)
        .await?
    {
        admin
    } else {
        admin || role_on_team_on(conn, team_id, caller.profile_id()).await? == Some(TeamRole::Owner)
    };
    if !admitted {
        return Err(ApiError::Forbidden);
    }
    if personal_of_on(conn, team_id).await?.is_some() {
        return Err(ApiError::BadRequest(
            "a personal team has no public_within policy".to_string(),
        ));
    }
    Ok(())
}

/// One effect transition `_public_within_transitions` reported.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
struct EffectChange {
    team_id: Uuid,
    anchor_id: Uuid,
    effective: bool,
}

#[derive(Debug, serde::Deserialize)]
struct PolicyWrite {
    outcome: String,
    #[serde(flatten)]
    effects: ActEffects,
}

/// Set or clear `team_id`'s `public_within` forbid (`kb_teams.public_within_forbidden`).
///
/// A forbid makes every row over this team, and over every team beneath it by any path, live or
/// dead, inert; a lower owner's own allow never lifts it. On the root it is the instance-wide off.
/// Authority is `policy_authority`, run on the pool and again under the team row's lock. The flag,
/// its `public_within_policy_changed` record and one `public_within_effect_changed` per row whose
/// effect flips are one statement (`_admin_public_within_policy_write`). The team row's lock is
/// what serializes it with a concurrent open, which holds a shared lock on every team whose forbid
/// reaches its team: an open is either refused by the forbid or recorded by it. The answer lists the rows made inert and the rows
/// revived, each named only to a caller who manages its team or anchor or is a system admin, and
/// counted otherwise.
pub async fn set_public_within_policy(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    forbidden: bool,
) -> ApiResult<PublicWithinPolicyOutcome> {
    {
        let admin = caller_is_admin(pool, caller).await?;
        let mut conn = pool.acquire().await?;
        policy_authority(&mut conn, caller, admin, team_id).await?;
    }
    let mut tx = pool.begin().await?;
    lock_teams_ascending(&mut tx, &[team_id]).await?;
    // The shared gating lock after the row, as every team door takes it: a repoint of the gating
    // slug cannot land between the re-check below and the write.
    sqlx::query_scalar!(r#"SELECT 1 AS "one!" FROM (SELECT _gating_team_lock(false)) g"#)
        .fetch_one(&mut *tx)
        .await?;
    let admin = caller_is_admin_on(&mut tx, caller).await?;
    policy_authority(&mut tx, caller, admin, team_id).await?;
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller.profile_id(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let raw = sqlx::query_scalar!(
        r#"SELECT _admin_public_within_policy_write($1, $2, $3, $4, NULL) AS "w!""#,
        emitter.uuid(),
        *caller.profile_id(),
        team_id,
        forbidden,
    )
    .fetch_one(&mut *tx)
    .await?;
    let write: PolicyWrite = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("_admin_public_within_policy_write: {e}")))?;
    if write.outcome == "absent" {
        return Err(ApiError::NotFound(
            "team not found or not readable".to_string(),
        ));
    }

    // A forbid never moves a link, so nothing lapses; the policy answer carries the flips only.
    let named = name_effects(&mut tx, caller, write.effects).await?;
    let outcome = PublicWithinPolicyOutcome {
        team_id,
        forbidden,
        changed: write.outcome == "changed",
        dormant: named.dormant,
        revived: named.revived,
        dormant_unnamed: named.dormant_unnamed,
        revived_unnamed: named.revived_unnamed,
    };
    tx.commit().await?;
    Ok(outcome)
}

/// The `public_within` rows that confer on the caller right now: effective rows anchored on a team
/// in the caller's closure, for a person (never a machine or a connection agent, which never read
/// through a row). An inert row, and a row anchored outside the closure, are absent.
pub async fn list_public_within(
    pool: &PgPool,
    caller: &HumanPrincipal,
) -> ApiResult<Vec<PublicWithinRowRef>> {
    let rows = sqlx::query!(
        r#"SELECT t.id AS "team_id!", t.slug AS "team_slug!", t.name AS "team_name!",
                  a.id AS "anchor_id!", a.slug AS "anchor_slug!", a.name AS "anchor_name!"
             FROM kb_team_public_within pw
             JOIN kb_teams t ON t.id = pw.team_id
             JOIN kb_teams a ON a.id = pw.anchor_id
            WHERE pw.anchor_id IN (SELECT r.team_id FROM profile_reachable_teams($1) r)
              AND public_within_reading($1)
              AND public_within_row_effective(pw.team_id, pw.anchor_id, pw.instance_wide)
            ORDER BY t.slug, a.slug"#,
        *caller.profile_id(),
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PublicWithinRowRef {
            team: TeamRef {
                id: r.team_id,
                slug: r.team_slug,
                name: r.team_name,
            },
            anchor: TeamRef {
                id: r.anchor_id,
                slug: r.anchor_slug,
                name: r.anchor_name,
            },
        })
        .collect())
}

/// The `public_within` rows with `team_id` as T, for its detail.
async fn public_within_of(pool: &PgPool, team_id: Uuid) -> ApiResult<Vec<PublicWithinRef>> {
    let rows = sqlx::query!(
        r#"SELECT a.id AS "anchor_id!", a.slug AS "anchor_slug!", a.name AS "anchor_name!",
                  public_within_row_effective(pw.team_id, pw.anchor_id, pw.instance_wide)
                      AS "effective!",
                  pw.instance_wide, pw.opened_at,
                  au.teams AS "teams!", au.principals AS "principals!"
             FROM kb_team_public_within pw
             JOIN kb_teams a ON a.id = pw.anchor_id
             CROSS JOIN LATERAL public_within_audience(pw.anchor_id) au
            WHERE pw.team_id = $1
            ORDER BY a.slug"#,
        team_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PublicWithinRef {
            anchor: TeamRef {
                id: r.anchor_id,
                slug: r.anchor_slug,
                name: r.anchor_name,
            },
            effective: r.effective,
            instance_wide: r.instance_wide,
            opened_at: r.opened_at,
            audience: PublicWithinAudience {
                teams: r.teams,
                principals: r.principals,
            },
        })
        .collect())
}

/// The `public_within` rows with `anchor_id` as A, for its detail.
async fn public_within_in_of(pool: &PgPool, anchor_id: Uuid) -> ApiResult<Vec<PublicWithinInRef>> {
    let rows = sqlx::query!(
        r#"SELECT t.id AS "team_id!", t.slug AS "team_slug!", t.name AS "team_name!",
                  public_within_row_effective(pw.team_id, pw.anchor_id, pw.instance_wide)
                      AS "effective!"
             FROM kb_team_public_within pw
             JOIN kb_teams t ON t.id = pw.team_id
            WHERE pw.anchor_id = $1
            ORDER BY t.slug"#,
        anchor_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PublicWithinInRef {
            team: TeamRef {
                id: r.team_id,
                slug: r.team_slug,
                name: r.team_name,
            },
            effective: r.effective,
        })
        .collect())
}

#[cfg(test)]
mod message_tests {
    /// C11 (team-role rank spec): no refusal, doc or published description in the member acts'
    /// files directs a caller to a transfer operation for team ownership — there is none; the
    /// hand-over is `change_role`. The invitation files joined when C2 routed invitations
    /// through the rank rule, and the MCP team tools and the CLI's teams skill joined when C5
    /// closed the inventory (plan C5). Files outside `crates/` stay out: a docs-only change does
    /// not run the Rust tests (`detect-ci-scope.sh`), so a test reading one would gate nothing.
    #[test]
    fn no_member_act_string_names_an_ownership_transfer() {
        let banned = [
            format!("ownership {}", "transfer"),
            format!("{} ownership", "transfer"),
            format!("ownership is {}", "transferred"),
        ];
        for (name, text) in [
            ("team_service.rs", include_str!("team_service.rs")),
            (
                "invitation_service.rs",
                include_str!("invitation_service.rs"),
            ),
            (
                "handlers/teams.rs",
                include_str!("../../../temper-api/src/handlers/teams.rs"),
            ),
            (
                "handlers/invitations.rs",
                include_str!("../../../temper-api/src/handlers/invitations.rs"),
            ),
            (
                "types/invitation.rs",
                include_str!("../../../temper-core/src/types/invitation.rs"),
            ),
            (
                "types/profile.rs",
                include_str!("../../../temper-core/src/types/profile.rs"),
            ),
            (
                "temper-mcp tools/invitations.rs",
                include_str!("../../../temper-mcp/src/tools/invitations.rs"),
            ),
            (
                "temper-mcp tools/teams.rs",
                include_str!("../../../temper-mcp/src/tools/teams.rs"),
            ),
            (
                "skill-content/teams.md",
                include_str!("../../../temper-cli/skill-content/teams.md"),
            ),
        ] {
            let lower = text.to_lowercase();
            for phrase in &banned {
                assert!(!lower.contains(phrase.as_str()), "{name} says {phrase:?}");
            }
        }
    }
}

#[cfg(all(test, feature = "test-db"))]
mod lifecycle_tests;
