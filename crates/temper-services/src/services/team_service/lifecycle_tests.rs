use super::*;
use sqlx::PgPool;
use temper_core::types::team::{TeamMemberSource, TeamRole};

/// Insert a profile with the given handle, return its id. It gets the `<handle>@web` emitter
/// entity the first authenticated request would provision, so its member acts can be recorded.
async fn mk_profile(pool: &PgPool, handle: &str) -> Uuid {
    let id = crate::test_support::profile::insert(pool, handle).await;
    crate::test_support::entity::insert(pool, id, &format!("{handle}@web")).await;
    id
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
    // owner removes `leaver`, who owns 2 resources in a context the team OWNS + 0 elsewhere. A
    // context merely shared to the team is out of its scope (team-authority fixes D5).
    let owner = mk_profile(&pool, "owner").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let carol = mk_profile(&pool, "carol").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, leaver, "member", "native").await;

    // Out of scope: carol's context shared to the team.
    let foreign = mk_shared_profile_context(&pool, carol, team, "carols").await;
    let _r0 = mk_homed_resource(&pool, foreign, leaver).await;
    let shared = mk_team_context(&pool, team, "shared").await;
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
async fn remove_member_residual_groups_by_context_identity(pool: PgPool) {
    // Two of the team's own contexts, both holding leaver-owned resources, inserted interleaved
    // (A, B, A). The fold must group by context IDENTITY (`context_id`), not by row adjacency.
    // (The scope is the team's OWN contexts since team-authority fixes D5, and a slug is unique
    // per owner, so two in-scope contexts can no longer share a slug; the grouping-by-identity
    // rule is still what this pins.)
    let owner = mk_profile(&pool, "owner").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, leaver, "member", "native").await;

    let ctx_a = mk_team_context(&pool, team, "docs-a").await;
    let ctx_b = mk_team_context(&pool, team, "docs-b").await;

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
    assert_eq!(by_ref.get("+acme/docs-a"), Some(&2));
    assert_eq!(by_ref.get("+acme/docs-b"), Some(&1));
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

/// C1, bite: an owner grants `owner` through a role change.
#[sqlx::test(migrations = "../../migrations")]
async fn owner_grants_owner_via_role_change(pool: PgPool) {
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
        TeamRole::Owner,
    )
    .await
    .expect("C1: an owner may grant owner");
    assert!(matches!(row.role, TeamRole::Owner));
    let events = membership_events(&pool, team).await;
    assert_eq!(events.len(), 1);
    assert_event(
        &events[0],
        member,
        Some("member"),
        Some("owner"),
        owner,
        "member_act",
    );
}

/// C1, pin: a maintainer cannot grant `owner`, and can grant up to `maintainer`.
#[sqlx::test(migrations = "../../migrations")]
async fn maintainer_grants_up_to_maintainer_and_not_owner(pool: PgPool) {
    let maintainer = mk_profile(&pool, "maint").await;
    let member = mk_profile(&pool, "member").await;
    let newcomer = mk_profile(&pool, "newcomer").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, maintainer, "maintainer", "native").await;
    add(&pool, team, member, "member", "native").await;
    let me = crate::test_support::human_principal_for(&pool, maintainer).await;

    let denied = change_role(&pool, &me, team, member, TeamRole::Owner).await;
    assert!(
        matches!(denied, Err(ApiError::ForbiddenDetail(_))),
        "got {denied:?}"
    );
    assert!(
        membership_events(&pool, team).await.is_empty(),
        "a refusal records nothing"
    );

    change_role(&pool, &me, team, member, TeamRole::Maintainer)
        .await
        .expect("a maintainer may grant maintainer");
    add_member(
        &pool,
        &me,
        team,
        &AddMemberRequest {
            profile_id: newcomer,
            role: TeamRole::Maintainer,
        },
    )
    .await
    .expect("a maintainer may add at maintainer");
    assert_eq!(membership_events(&pool, team).await.len(), 2);
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

    // A non-admin owner of the root is refused before the root guard: the root's lifecycle is a
    // system admin's alone (team-authority fixes D4).
    let denied = delete_team(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner).await,
        sys,
    )
    .await;
    assert!(matches!(denied, Err(ApiError::Forbidden)), "{denied:?}");

    // And a system admin meets the root guard itself.
    crate::test_support::approved_admin(&pool, owner).await;
    let denied = delete_team(
        &pool,
        &crate::test_support::human_principal_for(&pool, owner).await,
        sys,
    )
    .await;
    assert!(matches!(denied, Err(ApiError::Conflict(_))), "{denied:?}");
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

// --- `add_member` and the rank (team-role rank spec C1; was the G3 Phase B2 D7 owner guard) ---

/// C1, bite: an owner can grant `owner` — the refusal that pointed at a "transfer" that never
/// existed is gone.
#[sqlx::test(migrations = "../../migrations")]
async fn owner_adds_a_member_at_owner(pool: PgPool) {
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
            role: TeamRole::Owner,
        },
    )
    .await
    .expect("an owner may grant owner");
    assert!(matches!(row.role, TeamRole::Owner));
    let events = membership_events(&pool, team).await;
    assert_eq!(events.len(), 1, "one add, one record");
    assert_event(
        &events[0],
        newcomer,
        None,
        Some("owner"),
        owner,
        "member_act",
    );
}

/// C1, pin: a maintainer cannot add anyone at `owner`.
#[sqlx::test(migrations = "../../migrations")]
async fn maintainer_cannot_add_at_owner(pool: PgPool) {
    let maintainer = mk_profile(&pool, "maint").await;
    let newcomer = mk_profile(&pool, "newcomer").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, maintainer, "maintainer", "native").await;

    let denied = add_member(
        &pool,
        &crate::test_support::human_principal_for(&pool, maintainer).await,
        team,
        &AddMemberRequest {
            profile_id: newcomer,
            role: TeamRole::Owner,
        },
    )
    .await;
    assert!(
        matches!(denied, Err(ApiError::ForbiddenDetail(_))),
        "got {denied:?}"
    );
    assert_eq!(member_count(&pool, team).await, 1);
    assert!(membership_events(&pool, team).await.is_empty());
}

/// An add over an existing member is never an upsert, even by an owner granting `owner`: the
/// role changes only through `change_role` (#1070, plan P13).
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
    assert!(
        matches!(&denied, Err(ApiError::Conflict(m)) if m.contains("set-role")),
        "an add over an existing row is the #1070 409 naming change_role, got {denied:?}"
    );
    assert!(membership_events(&pool, team).await.is_empty());

    // The refusal leaves the existing row untouched.
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
            parents: None,
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

// ─── The rank rule's witnesses (team-role rank spec witness table; plan B4) ─────────────────
//
// Each conjunct is its own test. Every success asserts exactly one `team_membership_changed`
// with its before/after, actor and `via`; every refusal and no-op asserts none (C8).

/// The `team_membership_changed` payloads whose subject is `team`, oldest first.
async fn membership_events(pool: &PgPool, team: Uuid) -> Vec<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e \
           JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_membership_changed' \
            AND e.\"references\" @> jsonb_build_array(jsonb_build_object( \
                  'rel', 'subject', \
                  'target', jsonb_build_object('kind', 'kb_teams', 'id', $1::uuid))) \
          ORDER BY e.id",
    )
    .bind(team)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Every `team_membership_changed`, whatever its team.
async fn all_membership_events(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_membership_changed'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

fn assert_event(
    event: &serde_json::Value,
    member: Uuid,
    previous: Option<&str>,
    role: Option<&str>,
    actor: Uuid,
    via: &str,
) {
    assert_eq!(event["profile_id"], serde_json::json!(member), "{event}");
    assert_eq!(
        event.get("previous_role").and_then(|v| v.as_str()),
        previous,
        "{event}"
    );
    assert_eq!(event.get("role").and_then(|v| v.as_str()), role, "{event}");
    assert_eq!(event["actor"], serde_json::json!(actor), "{event}");
    assert_eq!(event["via"], via, "{event}");
    // Every routed member act writes or acts on a native row; an idp row is refused (C7).
    assert_eq!(event["source"], "native", "{event}");
    assert!(event.get("left_ownerless").is_none(), "{event}");
}

async fn role_of(pool: &PgPool, team: Uuid, profile: Uuid) -> Option<TeamRole> {
    sqlx::query_scalar("SELECT role FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(team)
        .bind(profile)
        .fetch_optional(pool)
        .await
        .unwrap()
}

async fn owner_count(pool: &PgPool, team: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_team_members WHERE team_id = $1 AND role = 'owner'")
        .bind(team)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A team made the gating team.
async fn mk_gating_team(pool: &PgPool) -> Uuid {
    let team = mk_team(pool, "gate").await;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'gate'")
        .execute(pool)
        .await
        .unwrap();
    team
}

/// A system admin (approved standing + governance grant) with a `@web` emitter.
async fn mk_admin(pool: &PgPool, handle: &str) -> Uuid {
    let id = mk_profile(pool, handle).await;
    crate::test_support::approved_admin(pool, id).await;
    id
}

/// A machine principal: a profile with a `kb_machine_clients` row.
async fn mk_machine(pool: &PgPool, handle: &str, registrar: Uuid) -> Uuid {
    let id = mk_profile(pool, handle).await;
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, 'test', $2, $3)",
    )
    .bind(format!("client-{handle}"))
    .bind(id)
    .bind(registrar)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn principal(pool: &PgPool, id: Uuid) -> HumanPrincipal {
    crate::test_support::human_principal_for(pool, id).await
}

/// P7, bite: `create_team`'s creator row is a recorded act — one `team_genesis` event per
/// team, for a root team and for a child team (R9), each naming the creator as both actor and
/// member.
#[sqlx::test(migrations = "../../migrations")]
async fn create_team_records_one_team_genesis_event_for_a_root_and_a_child(pool: PgPool) {
    let creator = mk_profile(&pool, "creator").await;
    let req = TeamCreateRequest {
        slug: "root".to_string(),
        name: None,
        parent: None,
        parents: None,
        auto_join_role: None,
    };
    let root = create_team(&pool, &principal(&pool, creator).await, &req)
        .await
        .unwrap();
    let events = membership_events(&pool, root.id).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_event(
        &events[0],
        creator,
        None,
        Some("owner"),
        creator,
        "team_genesis",
    );

    // A child team (R9): the same shape, one event.
    let child_req = TeamCreateRequest {
        slug: "child".to_string(),
        name: None,
        parent: Some("root".to_string()),
        parents: None,
        auto_join_role: None,
    };
    let child = create_team(&pool, &principal(&pool, creator).await, &child_req)
        .await
        .unwrap();
    let events = membership_events(&pool, child.id).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_event(
        &events[0],
        creator,
        None,
        Some("owner"),
        creator,
        "team_genesis",
    );
    assert_eq!(owner_count(&pool, child.id).await, 1);
}

/// C2, bite: a maintainer cannot demote an owner (today only the last-owner guard stops it).
#[sqlx::test(migrations = "../../migrations")]
async fn maintainer_cannot_demote_an_owner(pool: PgPool) {
    let (o1, o2, mt) = (
        mk_profile(&pool, "o1").await,
        mk_profile(&pool, "o2").await,
        mk_profile(&pool, "mt").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, o1, "owner", "native").await;
    add(&pool, team, o2, "owner", "native").await;
    add(&pool, team, mt, "maintainer", "native").await;

    let denied = change_role(
        &pool,
        &principal(&pool, mt).await,
        team,
        o1,
        TeamRole::Member,
    )
    .await;
    assert!(
        matches!(denied, Err(ApiError::ForbiddenDetail(_))),
        "got {denied:?}"
    );
    assert_eq!(role_of(&pool, team, o1).await, Some(TeamRole::Owner));
    assert!(membership_events(&pool, team).await.is_empty());
}

/// C2, bite: a maintainer cannot remove an owner.
#[sqlx::test(migrations = "../../migrations")]
async fn maintainer_cannot_remove_an_owner(pool: PgPool) {
    let (o1, o2, mt) = (
        mk_profile(&pool, "o1").await,
        mk_profile(&pool, "o2").await,
        mk_profile(&pool, "mt").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, o1, "owner", "native").await;
    add(&pool, team, o2, "owner", "native").await;
    add(&pool, team, mt, "maintainer", "native").await;

    let denied = remove_member(&pool, &principal(&pool, mt).await, team, o1).await;
    assert!(
        matches!(denied, Err(ApiError::ForbiddenDetail(_))),
        "got {denied:?}"
    );
    assert_eq!(owner_count(&pool, team).await, 2);
    assert!(membership_events(&pool, team).await.is_empty());
}

/// C2/R6, bite: an owner cannot demote a co-owner.
#[sqlx::test(migrations = "../../migrations")]
async fn owner_cannot_demote_a_co_owner(pool: PgPool) {
    let (o1, o2) = (mk_profile(&pool, "o1").await, mk_profile(&pool, "o2").await);
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, o1, "owner", "native").await;
    add(&pool, team, o2, "owner", "native").await;

    let denied = change_role(
        &pool,
        &principal(&pool, o1).await,
        team,
        o2,
        TeamRole::Maintainer,
    )
    .await;
    assert!(
        matches!(denied, Err(ApiError::ForbiddenDetail(_))),
        "got {denied:?}"
    );
    assert_eq!(owner_count(&pool, team).await, 2);
    assert!(membership_events(&pool, team).await.is_empty());
}

/// C2/R6, bite: an owner cannot remove a co-owner.
#[sqlx::test(migrations = "../../migrations")]
async fn owner_cannot_remove_a_co_owner(pool: PgPool) {
    let (o1, o2) = (mk_profile(&pool, "o1").await, mk_profile(&pool, "o2").await);
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, o1, "owner", "native").await;
    add(&pool, team, o2, "owner", "native").await;

    let denied = remove_member(&pool, &principal(&pool, o1).await, team, o2).await;
    assert!(
        matches!(denied, Err(ApiError::ForbiddenDetail(_))),
        "got {denied:?}"
    );
    assert_eq!(owner_count(&pool, team).await, 2);
    assert!(membership_events(&pool, team).await.is_empty());
}

/// C2, pin — THE R6 TRAP: "strictly below" applies only to an owner target. A maintainer
/// demotes and removes another maintainer.
#[sqlx::test(migrations = "../../migrations")]
async fn maintainer_demotes_and_removes_a_maintainer(pool: PgPool) {
    let (owner, m1, m2) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "m1").await,
        mk_profile(&pool, "m2").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, m1, "maintainer", "native").await;
    add(&pool, team, m2, "maintainer", "native").await;
    let me = principal(&pool, m1).await;

    change_role(&pool, &me, team, m2, TeamRole::Member)
        .await
        .expect("maintainer demotes a maintainer");
    remove_member(&pool, &me, team, m2)
        .await
        .expect("and removes them");
    assert_eq!(role_of(&pool, team, m2).await, None);
    let events = membership_events(&pool, team).await;
    assert_eq!(events.len(), 2);
    assert_event(
        &events[0],
        m2,
        Some("maintainer"),
        Some("member"),
        m1,
        "member_act",
    );
    assert_event(&events[1], m2, Some("member"), None, m1, "member_act");
}

/// C3a, pin (must survive R6): the hand-over. An owner grants `owner`, then lowers themselves.
#[sqlx::test(migrations = "../../migrations")]
async fn owner_hands_over_by_lowering_themselves(pool: PgPool) {
    let (owner, heir) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "heir").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, heir, "member", "native").await;
    let me = principal(&pool, owner).await;

    change_role(&pool, &me, team, heir, TeamRole::Owner)
        .await
        .expect("grant owner");
    let row = change_role(&pool, &me, team, owner, TeamRole::Maintainer)
        .await
        .expect("an owner lowers themselves while another owner remains");
    assert!(matches!(row.role, TeamRole::Maintainer));
    assert_eq!(role_of(&pool, team, heir).await, Some(TeamRole::Owner));
    let events = membership_events(&pool, team).await;
    assert_eq!(events.len(), 2);
    assert_event(
        &events[1],
        owner,
        Some("owner"),
        Some("maintainer"),
        owner,
        "self",
    );
}

/// C3a, pin: no one raises their own role.
#[sqlx::test(migrations = "../../migrations")]
async fn maintainer_cannot_raise_themselves(pool: PgPool) {
    let (owner, mt) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "mt").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, mt, "maintainer", "native").await;

    let denied = change_role(
        &pool,
        &principal(&pool, mt).await,
        team,
        mt,
        TeamRole::Owner,
    )
    .await;
    assert!(
        matches!(denied, Err(ApiError::ForbiddenDetail(_))),
        "got {denied:?}"
    );
    assert!(membership_events(&pool, team).await.is_empty());
}

/// C3, pin: members and watchers change no one, and may leave.
#[sqlx::test(migrations = "../../migrations")]
async fn members_and_watchers_change_no_one_but_may_leave(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let (member, watcher, other) = (
        mk_profile(&pool, "member").await,
        mk_profile(&pool, "watcher").await,
        mk_profile(&pool, "other").await,
    );
    let newcomer = mk_profile(&pool, "newcomer").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, member, "member", "native").await;
    add(&pool, team, watcher, "watcher", "native").await;
    add(&pool, team, other, "watcher", "native").await;

    for actor in [member, watcher] {
        let me = principal(&pool, actor).await;
        let add_req = AddMemberRequest {
            profile_id: newcomer,
            role: TeamRole::Watcher,
        };
        assert!(matches!(
            add_member(&pool, &me, team, &add_req).await,
            Err(ApiError::Forbidden)
        ));
        assert!(matches!(
            change_role(&pool, &me, team, other, TeamRole::Watcher).await,
            Err(ApiError::Forbidden)
        ));
        assert!(matches!(
            remove_member(&pool, &me, team, other).await,
            Err(ApiError::Forbidden)
        ));
    }
    assert!(membership_events(&pool, team).await.is_empty());

    for actor in [member, watcher] {
        remove_member(&pool, &principal(&pool, actor).await, team, actor)
            .await
            .expect("leave");
    }
    let events = membership_events(&pool, team).await;
    assert_eq!(events.len(), 2);
    assert_event(&events[0], member, Some("member"), None, member, "self");
}

/// P14 (c), C8: a self-change to your current role is a 200 and records nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn self_change_to_current_role_is_a_noop(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;

    let row = change_role(
        &pool,
        &principal(&pool, owner).await,
        team,
        owner,
        TeamRole::Owner,
    )
    .await
    .expect("a no-op is a 200");
    assert!(matches!(row.role, TeamRole::Owner));
    assert!(
        membership_events(&pool, team).await.is_empty(),
        "a change that did not happen is not recorded"
    );
}

/// C8: a manager's change to the target's current role is a 200 and records nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn change_to_the_current_role_records_nothing(pool: PgPool) {
    let (owner, member) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "member").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, member, "member", "native").await;

    change_role(
        &pool,
        &principal(&pool, owner).await,
        team,
        member,
        TeamRole::Member,
    )
    .await
    .expect("200");
    assert!(membership_events(&pool, team).await.is_empty());
}

/// P14 (b), Q4: on the gating team a member may leave, but may not lower themselves.
#[sqlx::test(migrations = "../../migrations")]
async fn gating_team_member_may_leave_but_not_lower_themselves(pool: PgPool) {
    let gate = mk_gating_team(&pool).await;
    let (o1, o2, member) = (
        mk_profile(&pool, "o1").await,
        mk_profile(&pool, "o2").await,
        mk_profile(&pool, "member").await,
    );
    add(&pool, gate, o1, "owner", "native").await;
    add(&pool, gate, o2, "owner", "native").await;
    add(&pool, gate, member, "member", "native").await;

    let denied = change_role(
        &pool,
        &principal(&pool, o1).await,
        gate,
        o1,
        TeamRole::Maintainer,
    )
    .await;
    assert!(
        matches!(denied, Err(ApiError::ForbiddenDetail(_))),
        "got {denied:?}"
    );
    assert!(membership_events(&pool, gate).await.is_empty());

    remove_member(&pool, &principal(&pool, o1).await, gate, o1)
        .await
        .expect("an owner may leave the gating team");
    remove_member(&pool, &principal(&pool, member).await, gate, member)
        .await
        .expect("a member may leave it");
    assert_eq!(membership_events(&pool, gate).await.len(), 2);
}

/// C14, bite: a gating-team owner who is not a system admin is refused on add, change and
/// remove of another.
#[sqlx::test(migrations = "../../migrations")]
async fn gating_team_owner_who_is_not_admin_is_refused(pool: PgPool) {
    let gate = mk_gating_team(&pool).await;
    let (owner, member, newcomer) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "member").await,
        mk_profile(&pool, "newcomer").await,
    );
    add(&pool, gate, owner, "owner", "native").await;
    add(&pool, gate, member, "member", "native").await;
    let me = principal(&pool, owner).await;

    let add_req = AddMemberRequest {
        profile_id: newcomer,
        role: TeamRole::Watcher,
    };
    assert!(matches!(
        add_member(&pool, &me, gate, &add_req).await,
        Err(ApiError::Forbidden)
    ));
    assert!(matches!(
        change_role(&pool, &me, gate, member, TeamRole::Watcher).await,
        Err(ApiError::Forbidden)
    ));
    assert!(matches!(
        remove_member(&pool, &me, gate, member).await,
        Err(ApiError::Forbidden)
    ));
    assert_eq!(member_count(&pool, gate).await, 2);
    assert!(membership_events(&pool, gate).await.is_empty());
}

/// Team DAG review G14, bite: with the gating team pointed elsewhere, `temper-system`'s roster
/// is still admin-only. Its owner who is not a system admin can neither add, promote, nor
/// remove another, nor invite onto it; a system admin still can.
#[sqlx::test(migrations = "../../migrations")]
async fn the_root_roster_is_admin_only_when_it_is_not_the_gating_team(pool: PgPool) {
    let _gate = mk_gating_team(&pool).await;
    let root: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (owner, member, newcomer) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "member").await,
        mk_profile(&pool, "newcomer").await,
    );
    for (who, role) in [(owner, "owner"), (member, "member")] {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, $3::team_role) \
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
        )
        .bind(root)
        .bind(who)
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(root)
        .bind(newcomer)
        .execute(&pool)
        .await
        .unwrap();
    let me = principal(&pool, owner).await;

    let add_req = AddMemberRequest {
        profile_id: newcomer,
        role: TeamRole::Watcher,
    };
    assert!(matches!(
        add_member(&pool, &me, root, &add_req).await,
        Err(ApiError::Forbidden)
    ));
    assert!(matches!(
        change_role(&pool, &me, root, member, TeamRole::Maintainer).await,
        Err(ApiError::Forbidden)
    ));
    assert!(matches!(
        remove_member(&pool, &me, root, member).await,
        Err(ApiError::Forbidden)
    ));
    assert!(membership_events(&pool, root).await.is_empty());
    let invite = crate::services::invitation_service::create_invitation(
        &pool,
        &me,
        root,
        crate::services::invitation_service::CreateInvitationParams {
            invited_email: "newcomer@example.com".to_string(),
            role: TeamRole::Watcher,
        },
    )
    .await;
    assert!(matches!(invite, Err(ApiError::Forbidden)), "{invite:?}");

    // Control: a system admin governs the root roster.
    let admin = mk_admin(&pool, "admin").await;
    change_role(
        &pool,
        &principal(&pool, admin).await,
        root,
        member,
        TeamRole::Watcher,
    )
    .await
    .expect("a system admin changes the root roster");
}

/// Team DAG review L9, bite: the database refuses a parent link that would make a team its own
/// ancestor — two-step and three-step — whoever writes it. A link that closes no cycle (a
/// second parent beside the first) is still written.
#[sqlx::test(migrations = "../../migrations")]
async fn a_team_cannot_be_linked_under_its_own_descendant(pool: PgPool) {
    let (a, b, c, d) = (
        mk_team(&pool, "a").await,
        mk_team(&pool, "b").await,
        mk_team(&pool, "c").await,
        mk_team(&pool, "d").await,
    );
    let link = |child: Uuid, parent: Uuid| {
        sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
            .bind(child)
            .bind(parent)
            .execute(&pool)
    };
    link(b, a).await.expect("b under a");
    link(c, b).await.expect("c under b");

    let two = link(a, b)
        .await
        .expect_err("a under b closes a two-step cycle");
    assert!(two.to_string().contains("its own descendants"), "{two}");
    let three = link(a, c)
        .await
        .expect_err("a under c closes a three-step cycle");
    assert!(three.to_string().contains("its own descendants"), "{three}");

    link(c, d)
        .await
        .expect("control: a second parent that closes no cycle");

    // The root takes no parent, which is what lets links under it skip the check.
    let root: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let parented_root = link(root, a).await.expect_err("the root takes no parent");
    assert!(
        parented_root.to_string().contains("takes no parent"),
        "{parented_root}"
    );
    link(d, root).await.expect("control: a link under the root");
}

/// Team DAG review G12, bite: `create_team` re-reads the parent role under the parent's row
/// lock. A creator demoted between the pool pre-check and the transaction is refused, and no
/// team is written. Simulated by holding the parent lock in another transaction, demoting the
/// creator there, and committing while `create_team` waits on it.
#[sqlx::test(migrations = "../../migrations")]
async fn create_team_rechecks_the_parent_role_under_its_lock(pool: PgPool) {
    let parent = mk_team(&pool, "parent").await;
    let creator = mk_profile(&pool, "creator").await;
    add(&pool, parent, creator, "maintainer", "native").await;
    let me = principal(&pool, creator).await;

    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(parent)
        .execute(&mut *blocker)
        .await
        .unwrap();

    let req = TeamCreateRequest {
        slug: "child".to_string(),
        name: None,
        parent: Some("parent".to_string()),
        parents: None,
        auto_join_role: None,
    };
    let pool2 = pool.clone();
    let attempt = tokio::spawn(async move { create_team(&pool2, &me, &req).await });

    // Wait until create_team is blocked on the parent lock — so its pool pre-check has already
    // passed, and only the re-check under the lock can refuse it — then demote under the lock.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if waiting >= 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "create_team never blocked on the parent lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    sqlx::query(
        "UPDATE kb_team_members SET role = 'watcher' WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(parent)
    .bind(creator)
    .execute(&mut *blocker)
    .await
    .unwrap();
    blocker.commit().await.unwrap();

    let outcome = attempt.await.unwrap();
    assert!(matches!(outcome, Err(ApiError::Forbidden)), "{outcome:?}");
    let child: Option<Uuid> = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'child'")
        .fetch_optional(&pool)
        .await
        .unwrap();
    assert!(child.is_none(), "no team may be written");
}

/// C14, bite: a system admin — not a member of it — changes the gating roster, on the ledger.
#[sqlx::test(migrations = "../../migrations")]
async fn admin_changes_the_gating_roster(pool: PgPool) {
    let gate = mk_gating_team(&pool).await;
    let admin = mk_admin(&pool, "admin").await;
    let (owner, newcomer) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "newcomer").await,
    );
    add(&pool, gate, owner, "owner", "native").await;
    let me = principal(&pool, admin).await;

    add_member(
        &pool,
        &me,
        gate,
        &AddMemberRequest {
            profile_id: newcomer,
            role: TeamRole::Owner,
        },
    )
    .await
    .expect("admin adds");
    change_role(&pool, &me, gate, owner, TeamRole::Member)
        .await
        .expect("admin changes an owner");
    remove_member(&pool, &me, gate, owner)
        .await
        .expect("admin removes");
    let events = membership_events(&pool, gate).await;
    assert_eq!(events.len(), 3);
    assert_event(
        &events[0],
        newcomer,
        None,
        Some("owner"),
        admin,
        "member_act",
    );
    assert_event(
        &events[1],
        owner,
        Some("owner"),
        Some("member"),
        admin,
        "member_act",
    );
    assert_event(&events[2], owner, Some("member"), None, admin, "member_act");
}

/// C9, bite: a system admin who is not a member repairs a team whose only owner is
/// deactivated, and the repair is recorded.
#[sqlx::test(migrations = "../../migrations")]
async fn system_admin_repairs_a_team_whose_only_owner_is_deactivated(pool: PgPool) {
    let admin = mk_admin(&pool, "admin").await;
    let (gone, heir) = (
        mk_profile(&pool, "gone").await,
        mk_profile(&pool, "heir").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, gone, "owner", "native").await;
    add(&pool, team, heir, "member", "native").await;
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'deactivated')")
        .bind(gone)
        .execute(&pool)
        .await
        .unwrap();
    let me = principal(&pool, admin).await;

    change_role(&pool, &me, team, heir, TeamRole::Owner)
        .await
        .expect("admin grants owner");
    remove_member(&pool, &me, team, gone)
        .await
        .expect("admin removes the departed owner");
    assert_eq!(role_of(&pool, team, heir).await, Some(TeamRole::Owner));
    assert_eq!(owner_count(&pool, team).await, 1);
    let events = membership_events(&pool, team).await;
    assert_eq!(events.len(), 2);
    assert_event(
        &events[0],
        heir,
        Some("member"),
        Some("owner"),
        admin,
        "member_act",
    );
    assert_event(&events[1], gone, Some("owner"), None, admin, "member_act");
}

/// C13, pin: owning a parent team confers nothing on its child.
#[sqlx::test(migrations = "../../migrations")]
async fn parent_owner_has_no_authority_on_the_child(pool: PgPool) {
    let (parent_owner, child_owner, member, newcomer) = (
        mk_profile(&pool, "po").await,
        mk_profile(&pool, "co").await,
        mk_profile(&pool, "m").await,
        mk_profile(&pool, "new").await,
    );
    let parent = mk_team(&pool, "parent").await;
    let child = mk_team(&pool, "child").await;
    sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
        .bind(child)
        .bind(parent)
        .execute(&pool)
        .await
        .unwrap();
    add(&pool, parent, parent_owner, "owner", "native").await;
    add(&pool, child, child_owner, "owner", "native").await;
    add(&pool, child, member, "member", "native").await;
    let me = principal(&pool, parent_owner).await;

    assert!(matches!(
        add_member(
            &pool,
            &me,
            child,
            &AddMemberRequest {
                profile_id: newcomer,
                role: TeamRole::Member,
            },
        )
        .await,
        Err(ApiError::Forbidden)
    ));
    assert!(matches!(
        change_role(&pool, &me, child, member, TeamRole::Watcher).await,
        Err(ApiError::Forbidden)
    ));
    assert!(matches!(
        remove_member(&pool, &me, child, member).await,
        Err(ApiError::Forbidden)
    ));
    assert!(membership_events(&pool, child).await.is_empty());
}

/// C7, pin: a member cannot leave their own identity-provider row; it changes only by
/// reconcile.
#[sqlx::test(migrations = "../../migrations")]
async fn idp_member_cannot_leave_their_idp_row(pool: PgPool) {
    let (owner, member) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "member").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, member, "member", "idp").await;

    let denied = remove_member(&pool, &principal(&pool, member).await, team, member).await;
    assert!(
        matches!(denied, Err(ApiError::Conflict(_))),
        "got {denied:?}"
    );
    assert!(membership_events(&pool, team).await.is_empty());
}

/// C7, pin: a manager cannot lower themselves on their own identity-provider row either.
#[sqlx::test(migrations = "../../migrations")]
async fn manager_cannot_lower_themselves_on_their_idp_row(pool: PgPool) {
    let (owner, mt) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "mt").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, mt, "maintainer", "idp").await;

    let denied = change_role(
        &pool,
        &principal(&pool, mt).await,
        team,
        mt,
        TeamRole::Member,
    )
    .await;
    assert!(
        matches!(denied, Err(ApiError::Conflict(_))),
        "got {denied:?}"
    );
    assert_eq!(role_of(&pool, team, mt).await, Some(TeamRole::Maintainer));
    assert!(membership_events(&pool, team).await.is_empty());
}

/// P14 (a), pin: a system admin with no row adds themselves and raises their own role — on the
/// ledger as `member_act`, the admin arm — and a later lowering of their own role is `self`.
#[sqlx::test(migrations = "../../migrations")]
async fn a_system_admin_adds_and_raises_themselves_on_the_ledger(pool: PgPool) {
    let admin = mk_admin(&pool, "admin").await;
    let owner = mk_profile(&pool, "owner").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    let me = principal(&pool, admin).await;

    add_member(
        &pool,
        &me,
        team,
        &AddMemberRequest {
            profile_id: admin,
            role: TeamRole::Member,
        },
    )
    .await
    .expect("an admin adds themselves");
    change_role(&pool, &me, team, admin, TeamRole::Owner)
        .await
        .expect("an admin raises their own role");
    change_role(&pool, &me, team, admin, TeamRole::Watcher)
        .await
        .expect("and lowers it");
    let events = membership_events(&pool, team).await;
    assert_eq!(events.len(), 3);
    assert_event(&events[0], admin, None, Some("member"), admin, "member_act");
    assert_event(
        &events[1],
        admin,
        Some("member"),
        Some("owner"),
        admin,
        "member_act",
    );
    assert_event(
        &events[2],
        admin,
        Some("owner"),
        Some("watcher"),
        admin,
        "self",
    );
}

/// C5, pin: an admin granting a machine `maintainer` gets the readable 400, not the trigger's
/// 500 — check 4 runs for every authority, before the database floor.
#[sqlx::test(migrations = "../../migrations")]
async fn admin_granting_a_machine_maintainer_gets_400_not_500(pool: PgPool) {
    let admin = mk_admin(&pool, "admin").await;
    let machine = mk_machine(&pool, "bot", admin).await;
    let team = mk_team(&pool, "acme").await;
    let me = principal(&pool, admin).await;

    let denied = add_member(
        &pool,
        &me,
        team,
        &AddMemberRequest {
            profile_id: machine,
            role: TeamRole::Maintainer,
        },
    )
    .await;
    assert!(
        matches!(denied, Err(ApiError::BadRequest(_))),
        "got {denied:?}"
    );
    add_member(
        &pool,
        &me,
        team,
        &AddMemberRequest {
            profile_id: machine,
            role: TeamRole::Member,
        },
    )
    .await
    .expect("member is within the ceiling");
    let denied = change_role(&pool, &me, team, machine, TeamRole::Maintainer).await;
    assert!(
        matches!(denied, Err(ApiError::BadRequest(_))),
        "got {denied:?}"
    );
    assert_eq!(membership_events(&pool, team).await.len(), 1);
}

/// A machine's personal team holds no owner, and nobody acts on it: a machine never acts on
/// membership (C4), and a system admin's arm does not reach a personal team (task `01a19006`).
#[sqlx::test(migrations = "../../migrations")]
async fn a_machine_personal_team_is_no_one_s_to_act_on(pool: PgPool) {
    let admin = mk_admin(&pool, "admin").await;
    let machine = mk_machine(&pool, "bot", admin).await;
    let human = mk_profile(&pool, "human").await;
    let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(machine)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        owner_count(&pool, personal).await,
        0,
        "a machine personal team has no owner"
    );
    let me = principal(&pool, admin).await;

    let denied = add_member(
        &pool,
        &me,
        personal,
        &AddMemberRequest {
            profile_id: human,
            role: TeamRole::Watcher,
        },
    )
    .await;
    assert!(matches!(denied, Err(ApiError::Forbidden)), "got {denied:?}");
    let denied = change_role(&pool, &me, personal, machine, TeamRole::Watcher).await;
    assert!(matches!(denied, Err(ApiError::Forbidden)), "got {denied:?}");
    assert!(membership_events(&pool, personal).await.is_empty());
}

/// #1070, pin: an add over any existing row is a 409 naming `change_role`, for every
/// authority — the admin's and an idp row's included.
#[sqlx::test(migrations = "../../migrations")]
async fn add_over_an_existing_member_is_409(pool: PgPool) {
    let admin = mk_admin(&pool, "admin").await;
    let (owner, member, provisioned) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "member").await,
        mk_profile(&pool, "idp").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, member, "member", "native").await;
    add(&pool, team, provisioned, "member", "idp").await;

    for actor in [owner, admin] {
        let me = principal(&pool, actor).await;
        for target in [member, provisioned] {
            let denied = add_member(
                &pool,
                &me,
                team,
                &AddMemberRequest {
                    profile_id: target,
                    role: TeamRole::Watcher,
                },
            )
            .await;
            assert!(
                matches!(&denied, Err(ApiError::Conflict(m)) if m.contains("set-role")),
                "got {denied:?}"
            );
        }
    }
    assert_eq!(role_of(&pool, team, member).await, Some(TeamRole::Member));
    assert!(membership_events(&pool, team).await.is_empty());
}

/// S6, pin: a caller with no authority learns nothing. A missing team, a missing member, a
/// present member, an idp row and a personal team all answer the same plain 403.
#[sqlx::test(migrations = "../../migrations")]
async fn non_manager_learns_nothing(pool: PgPool) {
    let (owner, member, provisioned, outsider) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "member").await,
        mk_profile(&pool, "idp").await,
        mk_profile(&pool, "outsider").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, member, "member", "native").await;
    add(&pool, team, provisioned, "member", "idp").await;
    let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(owner)
        .fetch_one(&pool)
        .await
        .unwrap();
    let missing_team = Uuid::now_v7();
    let missing = mk_profile(&pool, "nobody").await;

    for caller in [outsider, member] {
        let me = principal(&pool, caller).await;
        for (t, target) in [
            (missing_team, member),
            (team, missing),
            (team, owner),
            (team, provisioned),
            (personal, owner),
        ] {
            if target == caller {
                continue;
            }
            let r = remove_member(&pool, &me, t, target).await;
            assert!(
                matches!(r, Err(ApiError::Forbidden)),
                "remove {t}/{target} by {caller}: {r:?}"
            );
            let r = change_role(&pool, &me, t, target, TeamRole::Watcher).await;
            assert!(
                matches!(r, Err(ApiError::Forbidden)),
                "change {t}/{target} by {caller}: {r:?}"
            );
            let r = add_member(
                &pool,
                &me,
                t,
                &AddMemberRequest {
                    profile_id: target,
                    role: TeamRole::Watcher,
                },
            )
            .await;
            assert!(
                matches!(r, Err(ApiError::Forbidden)),
                "add {t}/{target} by {caller}: {r:?}"
            );
        }
    }
    // ...while a manager is told the truth about each.
    let me = principal(&pool, owner).await;
    assert!(matches!(
        remove_member(&pool, &me, team, missing).await,
        Err(ApiError::NotFound(_))
    ));
    assert!(matches!(
        remove_member(&pool, &me, team, provisioned).await,
        Err(ApiError::Conflict(_))
    ));
    assert_eq!(all_membership_events(&pool).await, 0);
}

/// C6, pin: the last owner can be neither demoted nor removed, by themselves or an admin.
#[sqlx::test(migrations = "../../migrations")]
async fn the_last_owner_is_guarded_for_every_authority(pool: PgPool) {
    let admin = mk_admin(&pool, "admin").await;
    let owner = mk_profile(&pool, "owner").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;

    for actor in [owner, admin] {
        let me = principal(&pool, actor).await;
        let r = change_role(&pool, &me, team, owner, TeamRole::Maintainer).await;
        assert!(
            matches!(&r, Err(ApiError::Conflict(m)) if m.contains("last owner")),
            "{r:?}"
        );
        let r = remove_member(&pool, &me, team, owner).await;
        assert!(
            matches!(&r, Err(ApiError::Conflict(m)) if m.contains("last owner")),
            "{r:?}"
        );
    }
    assert_eq!(owner_count(&pool, team).await, 1);
    assert!(membership_events(&pool, team).await.is_empty());
}

/// C6, BITE (finding 8): two co-owners leaving at the same moment leave exactly one owner.
///
/// The `race_the_scrub` holder pattern. A holds the team lock with its leave written but not
/// committed; B's leave is spawned and is shown to BLOCK on the team-row lock rather than
/// count the uncommitted state; A commits; B, re-resolving under the lock, sees itself as the
/// last owner and is refused. Before this build, B's leave ran unblocked against a count that
/// still saw A, and both commits left the team with no owner.
#[sqlx::test(migrations = "../../migrations")]
async fn two_co_owners_leaving_at_once_leave_one_owner(pool: PgPool) {
    let (a, b) = (mk_profile(&pool, "a").await, mk_profile(&pool, "b").await);
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, a, "owner", "native").await;
    add(&pool, team, b, "owner", "native").await;
    let a_principal = principal(&pool, a).await;
    let b_principal = principal(&pool, b).await;

    // Holder: A's leave, through the routed path, written and held open.
    let mut holder = pool.begin().await.unwrap();
    let target = team_membership::locked(&mut holder, team, ProfileId::from(a))
        .await
        .unwrap();
    let resolved = MembershipAuthority::resolve(&mut holder, &a_principal, team)
        .await
        .unwrap();
    let change = team_membership::authorize(resolved, MembershipAct::Remove, &target).unwrap();
    team_membership::membership_write(&mut holder, None, change)
        .await
        .unwrap();

    // Contender: B's leave.
    let contender_pool = pool.clone();
    let contender =
        tokio::spawn(async move { remove_member(&contender_pool, &b_principal, team, b).await });
    crate::test_support::await_team_lock_waiter(&pool).await;
    assert!(
        !contender.is_finished(),
        "B's leave must block on the team-row lock A holds, not run against a stale count"
    );

    holder.commit().await.unwrap();
    let b_result = contender.await.unwrap();
    assert!(
        matches!(&b_result, Err(ApiError::Conflict(m)) if m.contains("last owner")),
        "B, now the last owner, is refused: {b_result:?}"
    );
    assert_eq!(
        owner_count(&pool, team).await,
        1,
        "exactly one owner remains"
    );
    assert_eq!(role_of(&pool, team, b).await, Some(TeamRole::Owner));
    let events = membership_events(&pool, team).await;
    assert_eq!(events.len(), 1, "one leave happened, one is recorded");
    assert_event(&events[0], a, Some("owner"), None, a, "self");
}

/// P1, pin: the caller's own rank is read again under the team lock. An owner's demotion of
/// maintainer M is written and held open; M's act on a member blocks on the team-row lock, and
/// once the demotion commits M acts as the member they now are — refused, and nothing recorded.
#[sqlx::test(migrations = "../../migrations")]
async fn a_caller_demoted_mid_act_acts_on_their_new_role(pool: PgPool) {
    let (owner, mt, x) = (
        mk_profile(&pool, "owner").await,
        mk_profile(&pool, "mt").await,
        mk_profile(&pool, "x").await,
    );
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, mt, "maintainer", "native").await;
    add(&pool, team, x, "member", "native").await;
    let owner_principal = principal(&pool, owner).await;
    let mt_principal = principal(&pool, mt).await;

    // Holder: the owner demotes M, through the routed path, written and held open.
    let mut holder = pool.begin().await.unwrap();
    let target = team_membership::locked(&mut holder, team, ProfileId::from(mt))
        .await
        .unwrap();
    let resolved = MembershipAuthority::resolve(&mut holder, &owner_principal, team)
        .await
        .unwrap();
    let change = team_membership::authorize(
        resolved,
        MembershipAct::Change {
            role: TeamRole::Member,
        },
        &target,
    )
    .unwrap();
    team_membership::membership_write(&mut holder, None, change)
        .await
        .unwrap();

    // Contender: M, still a maintainer as the pool sees it, demotes X.
    let contender_pool = pool.clone();
    let contender = tokio::spawn(async move {
        change_role(&contender_pool, &mt_principal, team, x, TeamRole::Watcher).await
    });
    crate::test_support::await_team_lock_waiter(&pool).await;
    assert!(
        !contender.is_finished(),
        "M's act must block on the team-row lock, not decide on the rank the pool saw"
    );

    holder.commit().await.unwrap();
    let m_result = contender.await.unwrap();
    assert!(
        matches!(m_result, Err(ApiError::Forbidden)),
        "M, now a member, changes no one: {m_result:?}"
    );
    assert_eq!(role_of(&pool, team, x).await, Some(TeamRole::Member));
    let events = membership_events(&pool, team).await;
    assert_eq!(events.len(), 1, "only the demotion is recorded");
    assert_event(
        &events[0],
        mt,
        Some("maintainer"),
        Some("member"),
        owner,
        "member_act",
    );
}

// --- A personal team holds its owner and the owner's machines (task 01a19006) ---

/// Every member of `team`, by handle, for a refusal to compare against.
async fn roster(pool: &PgPool, team: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT p.handle FROM kb_team_members tm JOIN kb_profiles p ON p.id = tm.profile_id \
          WHERE tm.team_id = $1 ORDER BY p.handle",
    )
    .bind(team)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// A system admin cannot act on someone else's personal team through any membership door:
/// not add (themselves or anyone, at any role), not change the owner's role, not remove them.
/// The roster and the ledger are unchanged.
#[sqlx::test(migrations = "../../migrations")]
async fn a_system_admin_cannot_act_on_someone_elses_personal_team(pool: PgPool) {
    let admin = mk_admin(&pool, "admin").await;
    let dana = mk_profile(&pool, "dana").await;
    let other = mk_profile(&pool, "other").await;
    let (personal, _) = personal_team_of(&pool, dana).await;
    let me = principal(&pool, admin).await;

    for (who, role) in [
        (admin, TeamRole::Owner),
        (admin, TeamRole::Watcher),
        (other, TeamRole::Member),
    ] {
        let denied = add_member(
            &pool,
            &me,
            personal,
            &AddMemberRequest {
                profile_id: who,
                role,
            },
        )
        .await;
        assert!(
            matches!(denied, Err(ApiError::Forbidden)),
            "add {role:?}: {denied:?}"
        );
    }
    let denied = change_role(&pool, &me, personal, dana, TeamRole::Watcher).await;
    assert!(matches!(denied, Err(ApiError::Forbidden)), "got {denied:?}");
    let denied = remove_member(&pool, &me, personal, dana).await;
    assert!(matches!(denied, Err(ApiError::Forbidden)), "got {denied:?}");
    let denied = delete_team(&pool, &me, personal).await;
    assert!(matches!(denied, Err(ApiError::Forbidden)), "got {denied:?}");
    let root: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let denied = detach_child(&pool, &me, root, personal).await;
    assert!(matches!(denied, Err(ApiError::Forbidden)), "got {denied:?}");

    let live: bool = sqlx::query_scalar(
        "SELECT is_active AND EXISTS (SELECT 1 FROM kb_teams_parents \
          WHERE child_id = $1 AND parent_id = $2) FROM kb_teams WHERE id = $1",
    )
    .bind(personal)
    .bind(root)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(live, "still live, still under the root");
    assert_eq!(roster(&pool, personal).await, vec!["dana".to_string()]);
    assert!(membership_events(&pool, personal).await.is_empty());
}

/// Listing is not acting: a system admin still reads a personal team's roster, for audit.
#[sqlx::test(migrations = "../../migrations")]
async fn a_system_admin_still_lists_a_personal_team(pool: PgPool) {
    let admin = mk_admin(&pool, "admin").await;
    let dana = mk_profile(&pool, "dana").await;
    let (personal, slug) = personal_team_of(&pool, dana).await;

    let detail = team_detail(
        &pool,
        &crate::test_support::caller_for(&pool, admin).await,
        personal,
    )
    .await
    .expect("an admin lists a personal team");
    assert_eq!(detail.slug, slug);
    assert_eq!(detail.members.len(), 1);
}

/// The owner adds their own machine to their personal team; never another person, and never
/// someone else's machine — the service refuses by name, before the database would.
#[sqlx::test(migrations = "../../migrations")]
async fn the_owner_adds_their_machines_and_no_one_else(pool: PgPool) {
    let dana = mk_profile(&pool, "dana").await;
    let erin = mk_profile(&pool, "erin").await;
    let mine = mk_machine(&pool, "dana-bot", dana).await;
    let theirs = mk_machine(&pool, "erin-bot", erin).await;
    let (personal, _) = personal_team_of(&pool, dana).await;
    let me = principal(&pool, dana).await;

    add_member(
        &pool,
        &me,
        personal,
        &AddMemberRequest {
            profile_id: mine,
            role: TeamRole::Member,
        },
    )
    .await
    .expect("the owner's own machine is admitted");
    for who in [erin, theirs] {
        let denied = add_member(
            &pool,
            &me,
            personal,
            &AddMemberRequest {
                profile_id: who,
                role: TeamRole::Watcher,
            },
        )
        .await;
        assert!(
            matches!(&denied, Err(ApiError::BadRequest(m)) if m == PERSONAL_TEAM_HOLDS_ITS_OWNER),
            "got {denied:?}"
        );
    }
    assert_eq!(
        roster(&pool, personal).await,
        vec!["dana".to_string(), "dana-bot".to_string()]
    );
}

/// No team hangs under a personal team: its owner is refused by name, and nothing is created.
#[sqlx::test(migrations = "../../migrations")]
async fn no_child_team_under_a_personal_team(pool: PgPool) {
    let dana = mk_profile(&pool, "dana").await;
    let (_, slug) = personal_team_of(&pool, dana).await;

    let denied = create_team(
        &pool,
        &principal(&pool, dana).await,
        &TeamCreateRequest {
            slug: "dana-squad".into(),
            name: None,
            parent: Some(slug),
            parents: None,
            auto_join_role: None,
        },
    )
    .await;
    assert!(
        matches!(denied, Err(ApiError::BadRequest(_))),
        "got {denied:?}"
    );
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM kb_teams WHERE slug = 'dana-squad')")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!exists);
}

/// The database holds the rule for every writer the service does not front — hand-written
/// SQL here, standing in for an operator, a SAML reconcile or an older binary.
#[sqlx::test(migrations = "../../migrations")]
async fn the_database_refuses_what_the_service_refuses(pool: PgPool) {
    fn is_check_violation(err: &sqlx::Error) -> bool {
        err.as_database_error()
            .and_then(|e| e.code())
            .is_some_and(|code| code == "23514")
    }
    let dana = mk_profile(&pool, "dana").await;
    let erin = mk_profile(&pool, "erin").await;
    let (personal, _) = personal_team_of(&pool, dana).await;
    let shared = mk_team(&pool, "acme").await;

    let err = sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'watcher')",
    )
    .bind(personal)
    .bind(erin)
    .execute(&pool)
    .await
    .expect_err("another person on a personal team");
    assert!(is_check_violation(&err), "{err}");

    let err = sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
        .bind(shared)
        .bind(personal)
        .execute(&pool)
        .await
        .expect_err("a team under a personal team");
    assert!(is_check_violation(&err), "{err}");

    let err = sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
        .bind(personal)
        .bind(shared)
        .execute(&pool)
        .await
        .expect_err("a personal team under anything but temper-system");
    assert!(is_check_violation(&err), "{err}");

    let err = sqlx::query(
        "INSERT INTO kb_team_invitations (id, team_id, invited_email, invited_by_profile_id, \
                                          role, token) \
         VALUES (gen_random_uuid(), $1, 'erin@example.com', $2, 'member', 'tok')",
    )
    .bind(personal)
    .bind(dana)
    .execute(&pool)
    .await
    .expect_err("an invitation to a personal team");
    assert!(is_check_violation(&err), "{err}");

    sqlx::query("INSERT INTO kb_saml_idp (idp_key) VALUES ('okta')")
        .execute(&pool)
        .await
        .ok();
    let err = sqlx::query(
        "INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) \
         VALUES ('okta', 'eng', $1, 'member')",
    )
    .bind(personal)
    .execute(&pool)
    .await
    .expect_err("a SAML mapping onto a personal team");
    assert!(is_check_violation(&err), "{err}");

    // A shared team takes the same writes.
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'watcher')",
    )
    .bind(shared)
    .bind(erin)
    .execute(&pool)
    .await
    .expect("a shared team admits anyone");
    assert_eq!(roster(&pool, personal).await, vec!["dana".to_string()]);
}

/// A machine registration writes the machine's reach before its client row, in one
/// transaction; the membership rule is judged at commit, when the machine is its registrar's.
#[sqlx::test(migrations = "../../migrations")]
async fn a_machine_joins_its_registrars_personal_team_in_the_registering_transaction(pool: PgPool) {
    let dana = mk_profile(&pool, "dana").await;
    let erin = mk_profile(&pool, "erin").await;
    let (personal, _) = personal_team_of(&pool, dana).await;

    for (registrar, admitted) in [(dana, true), (erin, false)] {
        let mut tx = pool.begin().await.unwrap();
        let machine: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES (gen_random_uuid()::text, 'bot') \
             RETURNING id",
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(personal)
        .bind(machine)
        .execute(&mut *tx)
        .await
        .expect("not judged until commit");
        sqlx::query(
            "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
             VALUES (gen_random_uuid()::text, 'bot', $1, $2)",
        )
        .bind(machine)
        .bind(registrar)
        .execute(&mut *tx)
        .await
        .unwrap();
        assert_eq!(tx.commit().await.is_ok(), admitted, "registrar {registrar}");
    }
}
