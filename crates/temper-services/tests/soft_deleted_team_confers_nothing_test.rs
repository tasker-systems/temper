#![cfg(feature = "test-db")]

//! A soft-deleted team confers nothing (temper-artifacts
//! `specs/2026-10-31-soft-deleted-team-confers-nothing-design.md`): no authority over the team or its
//! property, no content it owns, and nothing is written onto it.
//!
//! ```text
//!   acme  (soft-deleted by alice once the fixture is built)
//!     alice:   owner          mallory: owner (a former manager once acme is dead)
//!     dave:    member, home owner of `doc` in `plans`
//!   plans: a context acme owns, shared to `partners` while acme was live
//!   partners: bob is a member
//!   carol: holds a read grant on `plans`
//!   loot: a team mallory creates after acme is dead
//! ```

use sqlx::PgPool;
use temper_core::types::connection::ProvisionConnectionRequest;
use temper_core::types::context::ShareContextRequest;
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};
use temper_services::error::ApiError;
use temper_services::services::{connection_service, context_service, team_service};
use temper_services::test_support;
use temper_workflow::operations::Surface;
use uuid::Uuid;

async fn person(pool: &PgPool, handle: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("seed profile");
    for surface in Surface::ALL {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, $2, '{}'::jsonb)",
        )
        .bind(id)
        .bind(format!("{handle}@{}", surface.marker()))
        .execute(pool)
        .await
        .expect("seed emitter");
    }
    test_support::approve(pool, id).await;
    id
}

async fn join(pool: &PgPool, team_id: Uuid, who: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team_id)
    .bind(who)
    .bind(role)
    .execute(pool)
    .await
    .expect("seed member");
}

async fn create_team(pool: &PgPool, creator: Uuid, slug: &str) -> Uuid {
    team_service::create_team(
        pool,
        &test_support::human_principal_for(pool, creator).await,
        &TeamCreateRequest {
            slug: slug.into(),
            name: None,
            parent: None,
            auto_join_role: None,
        },
    )
    .await
    .expect("create team")
    .id
}

async fn reads_context(pool: &PgPool, who: Uuid, context: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM contexts_readable_by($1) c WHERE c.context_id = $2)",
    )
    .bind(who)
    .bind(context)
    .fetch_one(pool)
    .await
    .expect("read probe")
}

async fn authors_context(pool: &PgPool, who: Uuid, context: Uuid) -> bool {
    sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
        .bind(who)
        .bind(context)
        .fetch_one(pool)
        .await
        .expect("author probe")
}

async fn sees_resource(pool: &PgPool, who: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(who)
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("resource probe")
}

async fn modifies_resource(pool: &PgPool, who: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(who)
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("modify probe")
}

struct Fixture {
    alice: Uuid,
    mallory: Uuid,
    bob: Uuid,
    carol: Uuid,
    dave: Uuid,
    acme: Uuid,
    partners: Uuid,
    plans: Uuid,
    doc: Uuid,
}

/// The fixture with acme still live; [`kill`] soft-deletes it through the real door.
async fn fixture(pool: &PgPool) -> Fixture {
    let alice = person(pool, "sd-alice").await;
    let mallory = person(pool, "sd-mallory").await;
    let bob = person(pool, "sd-bob").await;
    let carol = person(pool, "sd-carol").await;
    let dave = person(pool, "sd-dave").await;

    let acme = create_team(pool, alice, "sd-acme").await;
    join(pool, acme, mallory, "owner").await;
    join(pool, acme, dave, "member").await;
    let partners = create_team(pool, bob, "sd-partners").await;

    let plans: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, 'sd-plans', 'sd-plans') RETURNING id",
    )
    .bind(acme)
    .fetch_one(pool)
    .await
    .expect("acme's context");
    // Shared while acme was live (seeded: the share door also needs a manager of partners).
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(plans)
        .bind(partners)
        .execute(pool)
        .await
        .expect("plans shared to partners");
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, \
                principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true, $3)",
    )
    .bind(plans)
    .bind(carol)
    .bind(alice)
    .execute(pool)
    .await
    .expect("carol's grant on plans");

    let doc: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('doc', 'test://sd-doc') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("doc");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(doc)
    .bind(plans)
    .bind(dave)
    .execute(pool)
    .await
    .expect("doc's home");

    Fixture {
        alice,
        mallory,
        bob,
        carol,
        dave,
        acme,
        partners,
        plans,
        doc,
    }
}

async fn kill(pool: &PgPool, f: &Fixture) {
    team_service::delete_team(
        pool,
        &test_support::human_principal_for(pool, f.alice).await,
        f.acme,
    )
    .await
    .expect("alice soft-deletes acme");
}

/// D1/D2: a former manager administers none of a dead team's contexts. Each act is shown to work
/// while the team is live, then refused once it is dead. (The context-admin gate answers a refusal
/// `NotFound`, the same as a missing id; the two-sided gate answers `Forbidden`.)
///
/// FAILS IF `team_service::role_on_team` answers for a dead team again (share and retire are then
/// admitted), or `context_reassign`'s owner probe drops its `t.is_active` join.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_former_manager_administers_none_of_a_dead_teams_contexts(pool: PgPool) {
    let f = fixture(&pool).await;
    let loot = create_team(&pool, f.mallory, "sd-loot").await;
    let mallory = test_support::human_principal_for(&pool, f.mallory).await;
    let mallory_caller = test_support::caller_for(&pool, f.mallory).await;

    // Live control: the same act by the same person on a live team is admitted (and undone).
    context_service::share(
        &pool,
        &mallory,
        f.plans,
        &ShareContextRequest { team_id: loot },
    )
    .await
    .expect("a live manager may share");
    context_service::unshare(&pool, &mallory, f.plans, loot)
        .await
        .expect("and unshare");

    kill(&pool, &f).await;

    let share = context_service::share(
        &pool,
        &mallory,
        f.plans,
        &ShareContextRequest { team_id: loot },
    )
    .await;
    assert!(
        matches!(share, Err(ApiError::Forbidden)),
        "share of a dead team's context: {share:?}"
    );
    let reassign = context_service::reassign(&pool, &mallory, f.plans, loot).await;
    assert!(
        matches!(reassign, Err(ApiError::Forbidden)),
        "reassign of a dead team's context: {reassign:?}"
    );
    let retire = context_service::retire(&pool, &mallory_caller, f.plans).await;
    assert!(
        matches!(retire, Err(ApiError::NotFound(_))),
        "retire of a dead team's context: {retire:?}"
    );
    let rename = context_service::rename(&pool, &mallory_caller, f.plans, "renamed").await;
    assert!(
        matches!(rename, Err(ApiError::NotFound(_))),
        "rename of a dead team's context: {rename:?}"
    );
    assert!(
        !reads_context(&pool, loot_member(&pool, loot).await, f.plans).await,
        "nothing reached loot"
    );
}

/// A member of `loot` other than its creator, to show no reach landed there.
async fn loot_member(pool: &PgPool, loot: Uuid) -> Uuid {
    let eve = person(pool, "sd-eve").await;
    join(pool, loot, eve, "member").await;
    eve
}

/// D1/D2: restoring a dead team's retired context — which would put it back in front of every
/// live team it is shared to — is refused.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_former_manager_cannot_restore_a_dead_teams_retired_context(pool: PgPool) {
    let f = fixture(&pool).await;
    let mallory = test_support::caller_for(&pool, f.mallory).await;
    context_service::retire(&pool, &mallory, f.plans)
        .await
        .expect("a live manager retires the context");
    kill(&pool, &f).await;

    let restore = context_service::restore(&pool, &mallory, f.plans).await;
    assert!(
        matches!(restore, Err(ApiError::NotFound(_))),
        "restore of a dead team's context: {restore:?}"
    );
}

/// D3: a context a dead team owns reads as retired. A share and a grant made while the team was
/// live no longer reach it, and a write grant no longer authors it. What survives is what survives
/// retirement: the home owner of a resource in it keeps that resource.
///
/// FAILS IF `contexts_readable_by_teams` arms 3/4 or `context_authorable_by_profile`'s grant arm
/// lose `context_is_live` (bob or carol still read/author), or the home-owner arm is wrongly floored
/// (dave loses `doc`).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_dead_teams_context_reads_as_retired(pool: PgPool) {
    let f = fixture(&pool).await;
    assert!(
        reads_context(&pool, f.bob, f.plans).await,
        "precondition: the share reaches bob"
    );
    assert!(
        reads_context(&pool, f.carol, f.plans).await,
        "precondition: the grant reaches carol"
    );
    assert!(
        authors_context(&pool, f.carol, f.plans).await,
        "precondition: the write grant authors"
    );
    assert!(
        sees_resource(&pool, f.bob, f.doc).await,
        "precondition: bob sees doc"
    );

    kill(&pool, &f).await;

    assert!(
        !reads_context(&pool, f.bob, f.plans).await,
        "the share no longer reaches"
    );
    assert!(
        !reads_context(&pool, f.carol, f.plans).await,
        "the grant no longer reaches"
    );
    assert!(
        !authors_context(&pool, f.carol, f.plans).await,
        "the write grant no longer authors"
    );
    assert!(
        !sees_resource(&pool, f.bob, f.doc).await,
        "doc is not seen through the container"
    );
    assert!(
        !modifies_resource(&pool, f.carol, f.doc).await,
        "doc is not modified through the container"
    );
    assert!(
        !reads_context(&pool, f.dave, f.plans).await,
        "dave's membership of the dead owner confers nothing"
    );
    assert!(
        sees_resource(&pool, f.dave, f.doc).await,
        "the home owner keeps their resource"
    );
    assert!(
        modifies_resource(&pool, f.dave, f.doc).await,
        "and may still modify it"
    );
}

/// D3, the cognitive-map axis: a dead team's context reads as retired there too, which on this axis
/// means it stays reachable. `vis_team`'s share arm reaches a retired context's resources so a
/// steward can curate against its charter (`20260826000110_context_retirement.sql`), and a dead
/// owner is treated the same way. A map joined to `partners` keeps `doc` while bob, a person on
/// `partners`, loses it.
///
/// FAILS IF `vis_team`'s share arm gains an owner-liveness or `context_is_dead` floor (the map
/// loses `doc`) without the register entry being revisited, or the profile axis regains the share
/// (bob sees `doc`).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_cogmap_axis_reaches_a_dead_teams_context_as_a_retired_one(pool: PgPool) {
    let f = fixture(&pool).await;
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', 'test://sd-telos') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("telos");
    let map: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('sd-map', $1) RETURNING id",
    )
    .bind(telos)
    .fetch_one(&pool)
    .await
    .expect("map");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(map)
        .bind(f.partners)
        .execute(&pool)
        .await
        .expect("map joined to partners");
    let map_reaches = |resource: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM resources_accessible_to_cogmap($1) a \
                  WHERE a.resource_id = $2)",
            )
            .bind(map)
            .bind(resource)
            .fetch_one(&pool)
            .await
            .expect("cogmap probe")
        }
    };
    assert!(
        map_reaches(f.doc).await,
        "precondition: the map reaches doc"
    );

    kill(&pool, &f).await;

    assert!(
        map_reaches(f.doc).await,
        "the map still reaches doc in the dead team's context, as it would a retired one's"
    );
    assert!(
        !sees_resource(&pool, f.bob, f.doc).await,
        "a person on the same team does not"
    );
    assert!(
        !reads_context(&pool, f.bob, f.plans).await,
        "nor reads the context"
    );
}

/// D2: a system admin moves a dead team's context out to a live team, the recovery path, and it is
/// live again for that team.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_system_admin_reassigns_a_dead_teams_context_out(pool: PgPool) {
    let f = fixture(&pool).await;
    kill(&pool, &f).await;
    let admin = person(&pool, "sd-admin").await;
    test_support::approved_admin(&pool, admin).await;

    context_service::reassign(
        &pool,
        &test_support::human_principal_for(&pool, admin).await,
        f.plans,
        f.partners,
    )
    .await
    .expect("an admin reassigns the context out of the dead team");
    assert!(
        reads_context(&pool, f.bob, f.plans).await,
        "partners now owns it, so bob reads it"
    );
}

/// D4: nothing is written onto a dead team: not a member, not a share into it, not a reassign into
/// it, by a former owner or by a system admin.
///
/// FAILS IF `team_membership::lock_team` admits a dead team (the admin's add lands), or
/// `ensure_context_and_team_exist` / `context_reassign`'s target check do (the admin's share or
/// reassign into acme lands).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn nothing_is_written_onto_a_dead_team(pool: PgPool) {
    let f = fixture(&pool).await;
    kill(&pool, &f).await;
    let admin = person(&pool, "sd-admin").await;
    test_support::approved_admin(&pool, admin).await;
    let admin = test_support::human_principal_for(&pool, admin).await;
    let newcomer = person(&pool, "sd-newcomer").await;
    let add = AddMemberRequest {
        profile_id: newcomer,
        role: TeamRole::Member,
    };

    let by_owner = team_service::add_member(
        &pool,
        &test_support::human_principal_for(&pool, f.alice).await,
        f.acme,
        &add,
    )
    .await;
    assert!(
        by_owner.is_err(),
        "a former owner adds no one: {by_owner:?}"
    );
    let by_admin = team_service::add_member(&pool, &admin, f.acme, &add).await;
    assert!(
        matches!(by_admin, Err(ApiError::NotFound(_))),
        "an admin adds no one to a dead team: {by_admin:?}"
    );

    // A context on bob's live team: a system admin's arm does not reach someone else's personal
    // context (20261113000000), so a personal one would be refused before the dead-team check.
    let bobs: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, 'sd-bobs', 'sd-bobs') RETURNING id",
    )
    .bind(f.partners)
    .fetch_one(&pool)
    .await
    .expect("bob's context");
    let share = context_service::share(
        &pool,
        &admin,
        bobs,
        &ShareContextRequest { team_id: f.acme },
    )
    .await;
    assert!(
        matches!(share, Err(ApiError::NotFound(_))),
        "an admin shares nothing into a dead team: {share:?}"
    );
    let reassign = context_service::reassign(&pool, &admin, bobs, f.acme).await;
    assert!(reassign.is_err(), "nor reassigns into one: {reassign:?}");

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_team_members WHERE team_id = $1")
        .bind(f.acme)
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(rows, 3, "acme's roster is unchanged");
    let owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM kb_contexts WHERE id = $1")
        .bind(bobs)
        .fetch_one(&pool)
        .await
        .expect("owner");
    assert_eq!(owner, f.partners, "bob's team's context stayed its own");
}

/// D1: a former owner of a dead team controls none of its connections: they cannot grant a team
/// they run read reach on one.
///
/// FAILS IF `MachineRegistrationAuthority` answers on a dead team (via `role_on_team`).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_former_owner_grants_no_reach_on_a_dead_teams_connection(pool: PgPool) {
    let f = fixture(&pool).await;
    let mallory = test_support::human_principal_for(&pool, f.mallory).await;
    let conn = connection_service::provision(
        &pool,
        &mallory,
        &ProvisionConnectionRequest {
            provider: "github".into(),
            name: "sd acme hooks".into(),
            owner_team_id: Some(f.acme),
            reach_granularity: None,
            reach_covers: None,
        },
    )
    .await
    .expect("a live owner provisions a connection for acme")
    .id;
    let loot = create_team(&pool, f.mallory, "sd-loot").await;
    kill(&pool, &f).await;

    let reach = connection_service::grant_reach(&pool, &mallory, conn, loot, None).await;
    assert!(
        reach.is_err(),
        "a former owner grants no reach on a dead team's connection: {reach:?}"
    );
    let grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants \
          WHERE subject_table = 'kb_connections' AND subject_id = $1",
    )
    .bind(conn)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(grants, 0, "no reach row landed");
}
