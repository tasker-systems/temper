#![cfg(feature = "test-db")]

//! A private cognitive map is personal space (task `01a1933d`): one person per map, a system admin's
//! arm stops at it, and an admin it stops is told what they hit.
//!
//! ```text
//!   pat: a person; creates `pat-map` (unbound, so personal to pat)
//!   quinn: a person; holds can_grant on pat-map, granted by pat
//!   admin: a system admin, owner of admin-team (shared)
//! ```

use sqlx::PgPool;
use temper_core::types::cognitive_maps::{
    BindTeamRequest, GrantCapabilityRequest, RevokeCapabilityRequest,
};
use temper_core::types::context::ShareContextRequest;
use temper_core::types::ids::ProfileId;
use temper_services::backend::DbBackend;
use temper_services::error::ApiError;
use temper_services::services::{access_service, cogmap_service, context_service};
use temper_services::test_support;
use temper_workflow::operations::{Backend, CreateCognitiveMap, Surface};
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

async fn team_owned_by(pool: &PgPool, slug: &str, owner: Uuid) -> Uuid {
    let team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("seed team");
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')")
        .bind(team)
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    team
}

async fn personal_team(pool: &PgPool, profile: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("every profile has a personal team")
}

/// A real genesis by `creator`, through the backend: the self-grant and the creator column.
async fn genesis(pool: &PgPool, creator: Uuid, name: &str) -> Uuid {
    DbBackend::new(pool.clone(), ProfileId::from(creator))
        .create_cognitive_map(CreateCognitiveMap {
            request: temper_core::types::reconcile::CreateCogmapRequest {
                cogmap_id: None,
                telos_resource_id: None,
                name: name.to_string(),
                telos_title: format!("{name} telos"),
                telos: None,
            },
            origin: Surface::ApiHttp,
        })
        .await
        .expect("genesis a map")
        .value
        .cogmap_id
}

async fn owner_of(pool: &PgPool, map: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT cogmap_personal_owner($1)")
        .bind(map)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn bind_raw(pool: &PgPool, map: Uuid, team: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(map)
        .bind(team)
        .execute(pool)
        .await
        .map(|_| ())
}

fn cogmap_grant(map: Uuid, principal_table: &str, principal: Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_cogmaps".into(),
        subject_id: map,
        principal_table: principal_table.into(),
        principal_id: principal,
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
    }
}

fn says_personal_space(err: &ApiError, sentence_start: &str) -> bool {
    matches!(err, ApiError::ForbiddenDetail(m)
        if m.starts_with(sentence_start) && m.ends_with("the admin arm does not reach it"))
}

/// Genesis records its creator, and whose map it is follows the ruled order: a live shared binding
/// makes it nobody's; a soft-deleted one does not; the creator first, when a person.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn whose_map_it_is_follows_the_creator_and_live_shared_bindings(pool: PgPool) {
    let pat = person(&pool, "pat").await;
    let map = genesis(&pool, pat, "pat-map").await;

    let creator: Option<Uuid> =
        sqlx::query_scalar("SELECT created_by_profile_id FROM kb_cogmaps WHERE id = $1")
            .bind(map)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(creator, Some(pat), "genesis sets the creator");
    let from_ledger: Option<Uuid> = sqlx::query_scalar("SELECT cogmap_genesis_creator($1)")
        .bind(map)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(from_ledger, Some(pat), "the ledger says the same");
    assert_eq!(owner_of(&pool, map).await, Some(pat), "unbound and pat's");

    bind_raw(&pool, map, personal_team(&pool, pat).await)
        .await
        .expect("pat's own personal team");
    assert_eq!(owner_of(&pool, map).await, Some(pat));

    let shared = team_owned_by(&pool, "shared", pat).await;
    bind_raw(&pool, map, shared).await.unwrap();
    assert_eq!(owner_of(&pool, map).await, None, "a live shared binding");

    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(shared)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        owner_of(&pool, map).await,
        Some(pat),
        "a soft-deleted shared team confers nothing"
    );
}

/// A map with no creator (from before the grant chokepoint) is the person's of its one
/// personal-team binding, and may not gain a second.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_creatorless_map_is_its_one_personal_bindings_and_takes_no_second(pool: PgPool) {
    let pat = person(&pool, "pat").await;
    let quinn = person(&pool, "quinn").await;
    let map = genesis(&pool, pat, "legacy").await;
    sqlx::query("UPDATE kb_cogmaps SET created_by_profile_id = NULL WHERE id = $1")
        .bind(map)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(owner_of(&pool, map).await, None, "no creator, no binding");

    bind_raw(&pool, map, personal_team(&pool, quinn).await)
        .await
        .expect("one personal team");
    assert_eq!(owner_of(&pool, map).await, Some(quinn));

    let err = bind_raw(&pool, map, personal_team(&pool, pat).await)
        .await
        .expect_err("a second person's personal team is refused, to any writer");
    assert_eq!(
        err.as_database_error().and_then(|d| d.code()).as_deref(),
        Some("42501"),
        "{err}"
    );
}

/// The bind refusal holds for every caller, not only admins: quinn holds can_grant on pat's map and
/// manages their own personal team, so the two-sided gate admits them, and the one-person rule
/// refuses. The raw insert hits the trigger.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_map_is_bound_only_to_its_creators_personal_team(pool: PgPool) {
    let pat = person(&pool, "pat").await;
    let quinn = person(&pool, "quinn").await;
    let map = genesis(&pool, pat, "pat-map").await;
    let as_pat = test_support::human_principal_for(&pool, pat).await;
    let as_quinn = test_support::human_principal_for(&pool, quinn).await;
    let mut grant = cogmap_grant(map, "kb_profiles", quinn);
    grant.can_grant = true;
    access_service::grant_capability(&pool, &as_pat, &grant)
        .await
        .expect("pat shares grant authority with quinn");

    let refused = cogmap_service::bind_team(
        &pool,
        &as_quinn,
        map,
        &BindTeamRequest {
            team_id: personal_team(&pool, quinn).await,
        },
    )
    .await
    .expect_err("not pat's personal team");
    assert!(
        matches!(&refused, ApiError::ForbiddenDetail(m) if m.contains("only when that team's person created the map")),
        "{refused:?}"
    );
    let err = bind_raw(&pool, map, personal_team(&pool, quinn).await)
        .await
        .expect_err("the trigger refuses the same row");
    assert_eq!(
        err.as_database_error().and_then(|d| d.code()).as_deref(),
        Some("42501")
    );

    let bound = cogmap_service::bind_team(
        &pool,
        &as_pat,
        map,
        &BindTeamRequest {
            team_id: personal_team(&pool, pat).await,
        },
    )
    .await
    .expect("pat binds their own map to their own personal team");
    assert!(bound.bound);
}

/// A system admin does not grant on, bind, or unbind someone else's personal map, and each refusal
/// is a 403 that names the kind and the person. Its person keeps every act, and existing grants keep
/// conferring (R2). Once the map is bound to a live shared team, it is no one's and the admin arm
/// reaches it again.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_system_admin_is_stopped_at_someone_elses_personal_map_and_told_so(pool: PgPool) {
    let pat = person(&pool, "pat").await;
    let quinn = person(&pool, "quinn").await;
    let admin = person(&pool, "admin").await;
    test_support::approved_admin(&pool, admin).await;
    let admin_team = team_owned_by(&pool, "admin-team", admin).await;
    let map = genesis(&pool, pat, "pat-map").await;
    let as_pat = test_support::human_principal_for(&pool, pat).await;
    let as_admin = test_support::human_principal_for(&pool, admin).await;
    let pat_team = personal_team(&pool, pat).await;

    // Its person grants (R2: its person can add and revoke grants) and binds it to their own team.
    access_service::grant_capability(&pool, &as_pat, &cogmap_grant(map, "kb_profiles", quinn))
        .await
        .expect("pat grants on their own map");
    cogmap_service::bind_team(&pool, &as_pat, map, &BindTeamRequest { team_id: pat_team })
        .await
        .expect("pat binds it to their personal team");

    let cogmap = "this cognitive map is @pat's personal space";
    let refused = access_service::grant_capability(
        &pool,
        &as_admin,
        &cogmap_grant(map, "kb_profiles", admin),
    )
    .await
    .expect_err("an admin does not grant themselves read on it");
    assert!(says_personal_space(&refused, cogmap), "{refused:?}");
    let refused = access_service::revoke_capability(
        &pool,
        &as_admin,
        &RevokeCapabilityRequest {
            subject_table: "kb_cogmaps".into(),
            subject_id: map,
            principal_table: "kb_profiles".into(),
            principal_id: quinn,
        },
    )
    .await
    .expect_err("nor revoke what its person granted");
    assert!(says_personal_space(&refused, cogmap), "{refused:?}");
    let refused = cogmap_service::bind_team(
        &pool,
        &as_admin,
        map,
        &BindTeamRequest {
            team_id: admin_team,
        },
    )
    .await
    .expect_err("nor bind it to a team they manage");
    assert!(says_personal_space(&refused, cogmap), "{refused:?}");
    let refused = cogmap_service::unbind_team(&pool, &as_admin, map, pat_team)
        .await
        .expect_err("nor unbind it from its person's personal team");
    assert!(
        says_personal_space(&refused, "this team is @pat's personal team"),
        "{refused:?}"
    );

    let quinn_reads: bool =
        sqlx::query_scalar("SELECT can('kb_profiles', $1, 'read', 'kb_cogmaps', $2)")
            .bind(quinn)
            .bind(map)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(quinn_reads, "the grant its person made keeps conferring");
    access_service::revoke_capability(
        &pool,
        &as_pat,
        &RevokeCapabilityRequest {
            subject_table: "kb_cogmaps".into(),
            subject_id: map,
            principal_table: "kb_profiles".into(),
            principal_id: quinn,
        },
    )
    .await
    .expect("its person revokes");

    // Shared by its person: no longer personal space, and the admin arm reaches it.
    let shared = team_owned_by(&pool, "pat-shared", pat).await;
    cogmap_service::bind_team(&pool, &as_pat, map, &BindTeamRequest { team_id: shared })
        .await
        .expect("pat shares it through a shared team");
    access_service::grant_capability(&pool, &as_admin, &cogmap_grant(map, "kb_profiles", admin))
        .await
        .expect("a shared map is within the admin arm");
}

/// An admin's own personal map is theirs.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_system_admins_own_personal_map_is_theirs(pool: PgPool) {
    let admin = person(&pool, "admin").await;
    test_support::approved_admin(&pool, admin).await;
    let map = genesis(&pool, admin, "admin-map").await;
    let as_admin = test_support::human_principal_for(&pool, admin).await;
    let other = person(&pool, "other").await;
    access_service::grant_capability(&pool, &as_admin, &cogmap_grant(map, "kb_profiles", other))
        .await
        .expect("an admin grants on their own personal map");
    cogmap_service::bind_team(
        &pool,
        &as_admin,
        map,
        &BindTeamRequest {
            team_id: personal_team(&pool, admin).await,
        },
    )
    .await
    .expect("and binds it to their own personal team");
}

/// The context doors from #1275 say the same sentence: share and rename of someone else's personal
/// context answer a 403 that names it, where share was a bare 403 and rename a 404.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_context_doors_tell_an_admin_what_they_hit(pool: PgPool) {
    let pat = person(&pool, "pat").await;
    let admin = person(&pool, "admin").await;
    test_support::approved_admin(&pool, admin).await;
    let admin_team = team_owned_by(&pool, "admin-team", admin).await;
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, 'journal', 'journal') RETURNING id",
    )
    .bind(pat)
    .fetch_one(&pool)
    .await
    .unwrap();
    let as_admin = test_support::human_principal_for(&pool, admin).await;
    let admin_caller = test_support::caller_for(&pool, admin).await;

    let context = "this context is @pat's personal space";
    let refused = context_service::share(
        &pool,
        &as_admin,
        ctx,
        &ShareContextRequest {
            team_id: admin_team,
        },
    )
    .await
    .expect_err("share");
    assert!(says_personal_space(&refused, context), "{refused:?}");
    let refused = context_service::rename(&pool, &admin_caller, ctx, "x")
        .await
        .expect_err("rename");
    assert!(says_personal_space(&refused, context), "{refused:?}");
    let refused = context_service::retire(&pool, &admin_caller, ctx)
        .await
        .expect_err("retire");
    assert!(says_personal_space(&refused, context), "{refused:?}");

    // A non-admin stranger is unchanged: the context is invisible to them, and stays a 404.
    let stranger = person(&pool, "stranger").await;
    let stranger_caller = test_support::caller_for(&pool, stranger).await;
    let refused = context_service::rename(&pool, &stranger_caller, ctx, "x")
        .await
        .expect_err("rename by a stranger");
    assert!(matches!(refused, ApiError::NotFound(_)), "{refused:?}");
}

/// The shipped migration, executed verbatim by the two tests below.
const MIGRATION_SQL: &str =
    include_str!("../../../migrations/20261115000000_private_cogmaps_are_personal_space.sql");

/// `#[sqlx::test]` has already applied the migration. Undo it, so a test can stand in the world
/// before it and run it again.
async fn undo_migration(pool: &PgPool) {
    sqlx::raw_sql(
        "DROP TRIGGER kb_team_cogmaps_one_person ON kb_team_cogmaps;
         DROP FUNCTION kb_team_cogmaps_one_person();
         DROP FUNCTION cogmap_personal_bind_refusal(uuid, uuid);
         DROP FUNCTION cogmap_is_personal(uuid);
         DROP FUNCTION cogmap_personal_owner(uuid);
         DROP TRIGGER kb_access_grants_records_cogmap_creator ON kb_access_grants;
         DROP FUNCTION kb_access_grants_records_cogmap_creator();
         DROP FUNCTION cogmap_genesis_creator(uuid);
         ALTER TABLE kb_cogmaps DROP COLUMN created_by_profile_id;",
    )
    .execute(pool)
    .await
    .expect("undo the migration");
}

/// The backfill reads the genesis self-grant, and only that: a later self-grant on someone else's
/// map, an admin's, is not a creator.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_migration_backfills_the_creator_from_the_genesis_self_grant(pool: PgPool) {
    let pat = person(&pool, "pat").await;
    let admin = person(&pool, "admin").await;
    test_support::approved_admin(&pool, admin).await;
    let map = genesis(&pool, pat, "pat-map").await;
    // Before the migration, an admin could grant themselves on it.
    let other = genesis(&pool, admin, "admin-map").await;
    undo_migration(&pool).await;
    // The grant chokepoint itself, as the pre-migration admin arm reached it: the binary's door now
    // asks a function this test has just dropped.
    sqlx::query(
        "SELECT _admin_grant_created( \
           (SELECT id FROM kb_entities WHERE profile_id = $2 LIMIT 1), \
           'kb_cogmaps', $1, 'kb_profiles', $2, true, false, false, true, $2)",
    )
    .bind(map)
    .bind(admin)
    .execute(&pool)
    .await
    .expect("an admin self-grant on pat's map");

    sqlx::raw_sql(MIGRATION_SQL)
        .execute(&pool)
        .await
        .expect("the migration runs: no conflicts");
    for (m, who) in [(map, pat), (other, admin)] {
        let creator: Option<Uuid> =
            sqlx::query_scalar("SELECT created_by_profile_id FROM kb_cogmaps WHERE id = $1")
                .bind(m)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(creator, Some(who));
    }
}

/// R1: a map already bound to a personal team that is not its creator's, or to two people's, stops
/// the migration, and the error names the map and points at the pre-flight. After unbinding, it runs.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn existing_conflicts_stop_the_migration(pool: PgPool) {
    let pat = person(&pool, "pat").await;
    let quinn = person(&pool, "quinn").await;
    let map = genesis(&pool, pat, "pat-map").await;
    undo_migration(&pool).await;
    let quinn_team = personal_team(&pool, quinn).await;
    bind_raw(&pool, map, quinn_team).await.unwrap();

    let err = sqlx::raw_sql(MIGRATION_SQL)
        .execute(&pool)
        .await
        .expect_err("a map bound to someone else's personal team stops the migration");
    let message = err.to_string();
    assert!(
        message.contains(&map.to_string())
            && message.contains("bound to a personal team that is not its creator's")
            && message.contains("private-cogmaps-one-person-preflight.sql"),
        "{message}"
    );
    let column: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
          WHERE table_name = 'kb_cogmaps' AND column_name = 'created_by_profile_id')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!column, "the aborted migration changed nothing");

    sqlx::query("DELETE FROM kb_team_cogmaps WHERE cogmap_id = $1")
        .bind(map)
        .execute(&pool)
        .await
        .unwrap();
    bind_raw(&pool, map, personal_team(&pool, pat).await)
        .await
        .unwrap();
    sqlx::raw_sql(MIGRATION_SQL)
        .execute(&pool)
        .await
        .expect("unbound, the migration runs");
}

/// The machine-registration door confers cogmap grants too: a system admin provisioning a machine
/// with reach on someone else's personal map is stopped there and told so, and no machine is made.
/// On a shared map, and on their own, the admin's reach stays unchecked.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_system_admin_does_not_give_a_machine_reach_on_someone_elses_personal_map(pool: PgPool) {
    use temper_core::types::machine::{GrantSpec, ProvisionMachineRequest};
    use temper_services::auth_config::AuthMode;
    use temper_services::services::machine_registration_service;

    let pat = person(&pool, "pat").await;
    let admin = person(&pool, "admin").await;
    test_support::approved_admin(&pool, admin).await;
    let theirs = genesis(&pool, pat, "pat-map").await;
    let mine = genesis(&pool, admin, "admin-map").await;
    let as_admin = test_support::human_principal_for(&pool, admin).await;
    let req = |client: &str, map: Uuid| ProvisionMachineRequest {
        client_id: client.to_string(),
        label: client.to_string(),
        owner_team_id: None,
        teams: vec![],
        grants: vec![GrantSpec {
            cogmap_id: map,
            can_write: true,
        }],
    };

    let refused = machine_registration_service::provision(
        &pool,
        &as_admin,
        &req("bot-theirs", theirs),
        AuthMode::ExternalIdp,
    )
    .await
    .expect_err("no machine reach on pat's personal map");
    assert!(
        says_personal_space(&refused, "this cognitive map is @pat's personal space"),
        "{refused:?}"
    );
    let made: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_machine_clients WHERE client_id = 'bot-theirs'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(made, 0, "nothing was registered");

    machine_registration_service::provision(
        &pool,
        &as_admin,
        &req("bot-mine", mine),
        AuthMode::ExternalIdp,
    )
    .await
    .expect("an admin's own map is theirs to give reach on");
}

/// Genesis at an existing caller-supplied id (only a system admin's id is honoured) answers its
/// idempotent no-op only to a caller who reads the map: someone else's private map tells the admin
/// what they hit, a shared map they are not in is a bare 409, and their own map is the no-op
/// (ported from temper#1283, with the ruled 403 in place of its concealment).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn genesis_at_an_existing_id_answers_the_no_op_only_to_a_reader(pool: PgPool) {
    use temper_core::error::TemperError;
    let pat = person(&pool, "pat").await;
    let admin = person(&pool, "admin").await;
    test_support::approved_admin(&pool, admin).await;
    let theirs = genesis(&pool, pat, "pat-map").await;
    let shared = genesis(&pool, pat, "pat-shared").await;
    let pats_team = team_owned_by(&pool, "pats-team", pat).await;
    bind_raw(&pool, shared, pats_team).await.unwrap();
    let mine = genesis(&pool, admin, "admin-map").await;

    let at = |id: Uuid| {
        let pool = pool.clone();
        async move {
            DbBackend::new(pool, ProfileId::from(admin))
                .create_cognitive_map(CreateCognitiveMap {
                    request: temper_core::types::reconcile::CreateCogmapRequest {
                        cogmap_id: Some(id),
                        telos_resource_id: None,
                        name: "probe".to_string(),
                        telos_title: "probe".to_string(),
                        telos: None,
                    },
                    origin: Surface::ApiHttp,
                })
                .await
        }
    };

    match at(theirs).await {
        Err(TemperError::ForbiddenDetail(m)) => {
            assert!(
                m.starts_with("this cognitive map is @pat's personal space"),
                "{m}"
            )
        }
        other => panic!("someone else's private map: {other:?}"),
    }
    assert!(
        matches!(at(shared).await, Err(TemperError::Conflict(_))),
        "a shared map the admin is not in is only taken"
    );
    let noop = at(mine).await.expect("the admin's own map is the no-op");
    assert!(!noop.value.created);
}
