use super::*;

async fn mk_profile(pool: &PgPool, handle: &str) -> ProfileId {
    let id = crate::test_support::profile::insert(pool, handle).await;
    // Every profile needs a `<handle>@web` emitter entity for resolve_emitter.
    crate::test_support::entity::insert(pool, id, &format!("{handle}@web")).await;
    ProfileId::from(id)
}

async fn mk_context(pool: &PgPool, slug: &str, owner: ProfileId) -> Uuid {
    crate::test_support::context::insert_personal(pool, *owner, slug, slug).await
}

/// A context OWNED by `team` — the only kind a team's reassign reaches (team-authority fixes D5).
async fn mk_team_context(pool: &PgPool, slug: &str, team: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
         VALUES ($1, $1, 'kb_teams', $2) RETURNING id",
    )
    .bind(slug)
    .bind(team)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn mk_homed_resource(pool: &PgPool, ctx: Uuid, owner: ProfileId) -> Uuid {
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('r','r') RETURNING id",
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
    .bind(*owner)
    .execute(pool)
    .await
    .unwrap();
    rid
}

/// A resource homed in a cogmap (map interior). `anchor_id` needs no real cogmap row —
/// `kb_resource_homes.anchor_id` has no FK, and the reassign guard only inspects `anchor_table`.
async fn mk_cogmap_homed_resource(pool: &PgPool, owner: ProfileId) -> Uuid {
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('node','node') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', uuid_generate_v7(), $2, $2)",
    )
    .bind(rid)
    .bind(*owner)
    .execute(pool)
    .await
    .unwrap();
    rid
}

async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (gen_random_uuid(), $1, $1) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn add_member(pool: &PgPool, team: Uuid, p: ProfileId, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
         VALUES ($1,$2,$3::team_role,'native'::team_member_source)",
    )
    .bind(team)
    .bind(*p)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
}

async fn share_ctx(pool: &PgPool, ctx: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1,$2)")
        .bind(ctx)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();
}

async fn soft_delete_team(pool: &PgPool, team: Uuid) {
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(team)
        .execute(pool)
        .await
        .unwrap();
}

async fn retire_ctx(pool: &PgPool, ctx: Uuid) {
    sqlx::query("UPDATE kb_contexts SET is_active = false WHERE id = $1")
        .bind(ctx)
        .execute(pool)
        .await
        .unwrap();
}

async fn owner_of(pool: &PgPool, resource: Uuid) -> Uuid {
    sqlx::query_scalar!(
        "SELECT owner_profile_id FROM kb_resource_homes WHERE resource_id=$1",
        resource
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn visible_to(pool: &PgPool, profile: ProfileId, resource: Uuid) -> bool {
    sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id=$2) AS \"e!: bool\"",
        *profile,
        resource,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn team_scoped_owned_matches_bulk_scope(pool: PgPool) {
    // Same fixture shape as `bulk_reassigns_only_owned_and_scoped`.
    let leaver = mk_profile(&pool, "leaver").await;
    let other = mk_profile(&pool, "other").await;
    let team = mk_team(&pool, "acme").await;

    let shared = mk_team_context(&pool, "shared", team).await;
    let private = mk_context(&pool, "private", leaver).await; // NOT shared to team

    let in_scope = mk_homed_resource(&pool, shared, leaver).await; // owned + scoped → listed
    let _out_scope = mk_homed_resource(&pool, private, leaver).await; // owned, not scoped → excluded
    let _not_leaver = mk_homed_resource(&pool, shared, other).await; // scoped, other owner → excluded

    let rows = team_scoped_owned(&pool, team, *leaver)
        .await
        .expect("scope read");
    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.resource_id).collect();
    assert_eq!(ids, vec![in_scope]);
    assert!(
        rows[0].context_ref.ends_with("/shared"),
        "decorated ref: {}",
        rows[0].context_ref
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn owner_can_reassign_and_visibility_follows(pool: PgPool) {
    let alice = mk_profile(&pool, "alice").await;
    let bob = mk_profile(&pool, "bob").await;
    let ctx = mk_context(&pool, "alice-ctx", alice).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;

    reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *alice).await,
        r,
        *bob,
    )
    .await
    .expect("owner reassigns");

    assert_eq!(owner_of(&pool, r).await, *bob);
    assert!(visible_to(&pool, bob, r).await, "new owner sees it");
    assert!(
        visible_to(&pool, alice, r).await,
        "originator retains access"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn reassign_emits_event(pool: PgPool) {
    let alice = mk_profile(&pool, "alice").await;
    let bob = mk_profile(&pool, "bob").await;
    let ctx = mk_context(&pool, "c", alice).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;
    reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *alice).await,
        r,
        *bob,
    )
    .await
    .unwrap();
    let n = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
         WHERE t.name='resource_reassigned' AND (e.payload->>'resource_id')::uuid=$1",
        r,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn reassign_to_current_owner_is_noop(pool: PgPool) {
    let alice = mk_profile(&pool, "alice").await;
    let ctx = mk_context(&pool, "c", alice).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;
    reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *alice).await,
        r,
        *alice,
    )
    .await
    .expect("idempotent no-op");
    assert_eq!(owner_of(&pool, r).await, *alice);
}

#[sqlx::test(migrations = "../../migrations")]
async fn stranger_cannot_reassign(pool: PgPool) {
    let alice = mk_profile(&pool, "alice").await;
    let mallory = mk_profile(&pool, "mallory").await;
    let bob = mk_profile(&pool, "bob").await;
    let ctx = mk_context(&pool, "c", alice).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;
    let err = reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *mallory).await,
        r,
        *bob,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
}

/// FAILS IF the door answers a caller without authority differently by what the id is. An
/// unknown id, another's context-homed resource and another's cogmap-homed resource each refuse
/// the stranger the same `Forbidden` — no status names whether the id is a resource or what
/// kind of home it has. The bite: read the home with a `NotFound` for a missing row, or run the
/// cogmap-home `BadRequest` ahead of the authority check, in `reassign_resource`.
#[sqlx::test(migrations = "../../migrations")]
async fn a_stranger_is_refused_alike_whatever_the_id_is(pool: PgPool) {
    let alice = mk_profile(&pool, "alice").await;
    let mallory = mk_profile(&pool, "mallory").await;
    let ctx = mk_context(&pool, "c", alice).await;
    let in_context = mk_homed_resource(&pool, ctx, alice).await;
    let in_cogmap = mk_cogmap_homed_resource(&pool, alice).await;
    for (what, id) in [
        ("an unknown id", Uuid::now_v7()),
        ("another's context-homed resource", in_context),
        ("another's cogmap-homed resource", in_cogmap),
    ] {
        let err = reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *mallory).await,
            id,
            *mallory,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden), "{what}: {err:?}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn cannot_reassign_cogmap_homed_resource_even_as_owner(pool: PgPool) {
    let alice = mk_profile(&pool, "alice").await;
    let bob = mk_profile(&pool, "bob").await;
    let r = mk_cogmap_homed_resource(&pool, alice).await;
    let err = reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *alice).await,
        r,
        *bob,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::BadRequest(_)));
    assert_eq!(owner_of(&pool, r).await, *alice, "owner unchanged");
}

#[sqlx::test(migrations = "../../migrations")]
async fn team_admin_can_reassign_scoped_resource_to_member(pool: PgPool) {
    let alice = mk_profile(&pool, "alice").await; // current owner + departing
    let admin = mk_profile(&pool, "admin").await;
    let steward = mk_profile(&pool, "steward").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, steward, "member").await;
    add_member(&pool, team, alice, "member").await;
    let ctx = mk_team_context(&pool, "shared", team).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;

    reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        r,
        *steward,
    )
    .await
    .expect("admin reassigns to member");
    assert_eq!(owner_of(&pool, r).await, *steward);
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_cannot_reassign_to_non_member(pool: PgPool) {
    let alice = mk_profile(&pool, "alice").await;
    let admin = mk_profile(&pool, "admin").await;
    let outsider = mk_profile(&pool, "outsider").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, alice, "member").await;
    let ctx = mk_team_context(&pool, "shared", team).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;
    let err = reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        r,
        *outsider,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_cannot_reassign_unscoped_resource(pool: PgPool) {
    let alice = mk_profile(&pool, "alice").await;
    let admin = mk_profile(&pool, "admin").await;
    let steward = mk_profile(&pool, "steward").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, steward, "member").await;
    let ctx = mk_context(&pool, "private", alice).await; // NOT shared
    let r = mk_homed_resource(&pool, ctx, alice).await;
    let err = reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        r,
        *steward,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
}

#[sqlx::test(migrations = "../../migrations")]
async fn owner_reassign_to_nonexistent_profile_is_bad_request(pool: PgPool) {
    let alice = mk_profile(&pool, "alice").await;
    let ctx = mk_context(&pool, "c", alice).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;
    let ghost = Uuid::now_v7(); // never inserted
    let err = reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *alice).await,
        r,
        ghost,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::BadRequest(_)));
    assert_eq!(owner_of(&pool, r).await, *alice, "owner unchanged");
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_cannot_reassign_via_soft_deleted_team(pool: PgPool) {
    // Same shape as the passing admin case, but the team is soft-deleted → inert.
    let alice = mk_profile(&pool, "alice").await;
    let admin = mk_profile(&pool, "admin").await;
    let steward = mk_profile(&pool, "steward").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, alice, "member").await;
    add_member(&pool, team, steward, "member").await;
    let ctx = mk_team_context(&pool, "shared", team).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;
    soft_delete_team(&pool, team).await;

    let err = reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        r,
        *steward,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
    assert_eq!(owner_of(&pool, r).await, *alice, "owner unchanged");
}

#[sqlx::test(migrations = "../../migrations")]
async fn bulk_forbidden_on_soft_deleted_team(pool: PgPool) {
    let admin = mk_profile(&pool, "admin").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let steward = mk_profile(&pool, "steward").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, leaver, "member").await;
    add_member(&pool, team, steward, "member").await;
    let shared = mk_team_context(&pool, "shared", team).await;
    let r = mk_homed_resource(&pool, shared, leaver).await;
    soft_delete_team(&pool, team).await;

    let err = reassign_team_resources(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        team,
        *leaver,
        *steward,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
    assert_eq!(owner_of(&pool, r).await, *leaver, "owner unchanged");
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_cannot_reassign_via_retired_context(pool: PgPool) {
    // Same shape as the passing admin case, but the CONTEXT is retired. Retirement
    // revokes the team-share arm; the reassign must not hand it back as ownership.
    let alice = mk_profile(&pool, "alice").await;
    let admin = mk_profile(&pool, "admin").await;
    let steward = mk_profile(&pool, "steward").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, alice, "member").await;
    add_member(&pool, team, steward, "member").await;
    let ctx = mk_team_context(&pool, "shared", team).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;
    retire_ctx(&pool, ctx).await;

    let err = reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        r,
        *steward,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
    assert_eq!(owner_of(&pool, r).await, *alice, "owner unchanged");
}

#[sqlx::test(migrations = "../../migrations")]
async fn owner_can_still_reassign_out_of_a_retired_context(pool: PgPool) {
    // Anti-trap: retirement floors the ADMIN arm only. An owner must keep being able
    // to hand off their own work after the context is retired.
    let alice = mk_profile(&pool, "alice").await;
    let bob = mk_profile(&pool, "bob").await;
    let ctx = mk_context(&pool, "alice-ctx", alice).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;
    retire_ctx(&pool, ctx).await;

    reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *alice).await,
        r,
        *bob,
    )
    .await
    .expect("owner reassigns out of a retired context");
    assert_eq!(owner_of(&pool, r).await, *bob);
}

#[sqlx::test(migrations = "../../migrations")]
async fn bulk_excludes_a_retired_contexts_resources(pool: PgPool) {
    // The team is live, so the call is authorized and succeeds — with a reduced scope.
    let admin = mk_profile(&pool, "admin").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let steward = mk_profile(&pool, "steward").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, steward, "member").await;
    add_member(&pool, team, leaver, "member").await;

    let live = mk_team_context(&pool, "live", team).await;
    let retired = mk_team_context(&pool, "retired", team).await;

    let in_scope = mk_homed_resource(&pool, live, leaver).await; // live share → moves
    let out_scope = mk_homed_resource(&pool, retired, leaver).await; // retired → stays
    retire_ctx(&pool, retired).await;

    let moved = reassign_team_resources(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        team,
        *leaver,
        *steward,
    )
    .await
    .expect("live team, reduced scope");

    assert_eq!(moved, vec![in_scope]);
    assert_eq!(owner_of(&pool, in_scope).await, *steward);
    assert_eq!(owner_of(&pool, out_scope).await, *leaver, "retired stays");
}

#[sqlx::test(migrations = "../../migrations")]
async fn bulk_reassigns_only_owned_and_scoped(pool: PgPool) {
    let admin = mk_profile(&pool, "admin").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let steward = mk_profile(&pool, "steward").await;
    let other = mk_profile(&pool, "other").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, steward, "member").await;
    add_member(&pool, team, leaver, "member").await;

    let shared = mk_team_context(&pool, "shared", team).await;
    let private = mk_context(&pool, "private", leaver).await; // NOT shared to team

    let in_scope = mk_homed_resource(&pool, shared, leaver).await; // owned+scoped → moves
    let out_scope = mk_homed_resource(&pool, private, leaver).await; // owned, not scoped → stays
    let not_leaver = mk_homed_resource(&pool, shared, other).await; // scoped, other owner → stays

    let moved = reassign_team_resources(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        team,
        *leaver,
        *steward,
    )
    .await
    .expect("bulk reassign");

    assert_eq!(moved, vec![in_scope]);
    assert_eq!(owner_of(&pool, in_scope).await, *steward);
    assert_eq!(owner_of(&pool, out_scope).await, *leaver);
    assert_eq!(owner_of(&pool, not_leaver).await, *other);
}

#[sqlx::test(migrations = "../../migrations")]
async fn bulk_non_manager_forbidden(pool: PgPool) {
    let stranger = mk_profile(&pool, "stranger").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let steward = mk_profile(&pool, "steward").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, steward, "member").await;
    let err = reassign_team_resources(
        &pool,
        &crate::test_support::human_principal_for(&pool, *stranger).await,
        team,
        *leaver,
        *steward,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
}

#[sqlx::test(migrations = "../../migrations")]
async fn bulk_into_non_member_forbidden(pool: PgPool) {
    let admin = mk_profile(&pool, "admin").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let outsider = mk_profile(&pool, "outsider").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    let err = reassign_team_resources(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        team,
        *leaver,
        *outsider,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
}

#[sqlx::test(migrations = "../../migrations")]
async fn bulk_empty_match_is_ok(pool: PgPool) {
    let admin = mk_profile(&pool, "admin").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let steward = mk_profile(&pool, "steward").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, steward, "member").await;
    add_member(&pool, team, leaver, "member").await;
    let moved = reassign_team_resources(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        team,
        *leaver,
        *steward,
    )
    .await
    .expect("empty is ok");
    assert!(moved.is_empty());
}

// ---- Reassign acts only inside the team's own contexts (team-authority fixes D5). ----

#[sqlx::test(migrations = "../../migrations")]
async fn a_forced_member_cannot_be_reassigned_out_of_their_own_teams_shared_context(pool: PgPool) {
    // The shape that defeated the first D5: Bob owns team B, B's context is shared read-only
    // into T, and T's manager Mallory force-adds Bob to T (add_member needs no consent). Bob's
    // resource in B's context stays Bob's.
    let mallory = mk_profile(&pool, "mallory").await;
    let bob = mk_profile(&pool, "bob").await;
    let t = mk_team(&pool, "acme").await;
    let b = mk_team(&pool, "bobs").await;
    add_member(&pool, t, mallory, "owner").await;
    add_member(&pool, b, bob, "owner").await;
    let ctx = mk_team_context(&pool, "b-notes", b).await;
    share_ctx(&pool, ctx, t).await;
    let r = mk_homed_resource(&pool, ctx, bob).await;
    add_member(&pool, t, bob, "watcher").await;

    let m = crate::test_support::human_principal_for(&pool, *mallory).await;
    let err = reassign_resource(&pool, &m, r, *mallory).await.unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    let moved = reassign_team_resources(&pool, &m, t, *bob, *mallory)
        .await
        .expect("authorized, empty scope");
    assert!(moved.is_empty());
    assert_eq!(owner_of(&pool, r).await, *bob);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_shared_context_is_never_in_scope(pool: PgPool) {
    // Bob shares HIS context read-only into the team. Whether or not he is a member, the
    // team's managers cannot take his resources in it: a share confers read, never authority
    // over the shared container's contents.
    let admin = mk_profile(&pool, "admin").await;
    let bob = mk_profile(&pool, "bob").await;
    let mallory = mk_profile(&pool, "mallory").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, mallory, "maintainer").await;
    let ctx = mk_context(&pool, "bob-notes", bob).await;
    share_ctx(&pool, ctx, team).await;
    let r = mk_homed_resource(&pool, ctx, bob).await;

    // Not a member: refused outright, on both doors.
    let err = reassign_team_resources(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        team,
        *bob,
        *mallory,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    let err = reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        r,
        *mallory,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");

    // A member: still never his own context's resources.
    add_member(&pool, team, bob, "member").await;
    let moved = reassign_team_resources(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        team,
        *bob,
        *mallory,
    )
    .await
    .expect("authorized, empty scope");
    assert!(moved.is_empty());
    let err = reassign_resource(
        &pool,
        &crate::test_support::human_principal_for(&pool, *admin).await,
        r,
        *mallory,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert_eq!(owner_of(&pool, r).await, *bob);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_former_member_recorded_on_the_ledger_is_handed_off(pool: PgPool) {
    // Offboarding proper: the leaver is removed through the recorded membership write, then
    // their work in a TEAM-OWNED context (in scope since D5) is handed to a steward.
    let admin = mk_profile(&pool, "admin").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let steward = mk_profile(&pool, "steward").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, admin, "owner").await;
    add_member(&pool, team, steward, "member").await;
    add_member(&pool, team, leaver, "member").await;
    let team_ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, 'team-ctx', 'team-ctx') RETURNING id",
    )
    .bind(team)
    .fetch_one(&pool)
    .await
    .unwrap();
    let r = mk_homed_resource(&pool, team_ctx, leaver).await;

    let admin_p = crate::test_support::human_principal_for(&pool, *admin).await;
    crate::services::team_service::remove_member(&pool, &admin_p, team, *leaver)
        .await
        .expect("remove the leaver through the recorded write");

    let moved = reassign_team_resources(&pool, &admin_p, team, *leaver, *steward)
        .await
        .expect("a former member's work is handed off");
    assert_eq!(moved, vec![r]);
    assert_eq!(owner_of(&pool, r).await, *steward);
}
