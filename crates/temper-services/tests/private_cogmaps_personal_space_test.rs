#![cfg(feature = "test-db")]

//! Private cognitive maps are personal space, personal to at most one person (task `01a1933d`).
//!
//! - **Whose.** `cogmap_personal_owner` is NULL for a map bound to a live shared team; otherwise
//!   the map's creator (`kb_cogmaps.created_by_profile_id`, recorded from the genesis seed);
//!   otherwise the one person whose personal team it is bound to.
//! - **The admin arm stops there.** A system admin does not grant on, bind or unbind someone else's
//!   private map, nor grant on a resource homed in one, and is refused as on an id that names
//!   nothing, so the refusal does not confirm the id is personal space. The same `NotFound` answers
//!   an admin on someone else's personal context at share and reassign.
//! - **One person.** A binding to a person's personal team is refused for every caller unless that
//!   person created the map, or the map has no person creator and no other person's personal team.
//!
//! ```text
//!   owner: creates `map` (unbound) ─ personal team owner-personal
//!   other: another person          ─ personal team other-personal
//!   admin: a system admin, owner of the shared team admin-team
//! ```

use sqlx::PgPool;
use temper_core::types::cognitive_maps::{
    BindTeamRequest, GrantCapabilityRequest, RevokeCapabilityRequest,
};
use temper_core::types::context::ShareContextRequest;
use temper_core::types::ids::ProfileId;
use temper_services::backend::DbBackend;
use temper_services::error::ApiError;
use temper_services::services::{
    access_service, admin_ledger_service, cogmap_service, context_service,
};
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

async fn admin(pool: &PgPool, handle: &str) -> Uuid {
    let id = person(pool, handle).await;
    test_support::approved_admin(pool, id).await;
    id
}

/// The personal team profile genesis made for `profile`.
async fn personal_team(pool: &PgPool, profile: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("genesis made a personal team")
}

async fn shared_team(pool: &PgPool, slug: &str, owner: Uuid) -> Uuid {
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

/// A map born through genesis, as `creator`.
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
        .expect("genesis")
        .value
        .cogmap_id
}

async fn personal_owner(pool: &PgPool, cogmap: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT cogmap_personal_owner($1)")
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn bind_raw(pool: &PgPool, cogmap: Uuid, team: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .map(|_| ())
}

fn is_tc001(err: &sqlx::Error) -> bool {
    err.as_database_error().and_then(|e| e.code()).as_deref() == Some("TC001")
}

fn map_grant(cogmap: Uuid, team: Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_cogmaps".into(),
        subject_id: cogmap,
        principal_table: "kb_teams".into(),
        principal_id: team,
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
    }
}

#[track_caller]
fn assert_not_found(refused: &ApiError, expected: &str) {
    match refused {
        ApiError::NotFound(message) => assert_eq!(message, expected),
        other => panic!("expected NotFound({expected:?}), got {other:?}"),
    }
}

/// Genesis records the creator; a later self-grant, an admin's included, does not move it. An
/// unbound map is its creator's; a live shared binding makes it nobody's; a soft-deleted shared
/// team shares nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_private_map_is_its_creators_until_it_is_shared(pool: PgPool) {
    let owner = person(&pool, "pc-owner").await;
    let admin = admin(&pool, "pc-admin").await;
    let map = genesis(&pool, owner, "Private").await;

    let creator: Option<Uuid> =
        sqlx::query_scalar("SELECT created_by_profile_id FROM kb_cogmaps WHERE id = $1")
            .bind(map)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(creator, Some(owner), "genesis records the creator");
    assert_eq!(personal_owner(&pool, map).await, Some(owner));

    // A later self-grant on the map, written straight to the table as an admin with a database
    // shell could, is not the genesis seed and records nothing.
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
           can_read, granted_by_profile_id) VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, $2)",
    )
    .bind(map)
    .bind(admin)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(personal_owner(&pool, map).await, Some(owner));

    let shared = shared_team(&pool, "pc-shared", owner).await;
    bind_raw(&pool, map, shared).await.unwrap();
    assert_eq!(personal_owner(&pool, map).await, None, "shared on purpose");

    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(shared)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        personal_owner(&pool, map).await,
        Some(owner),
        "a soft-deleted team shares nothing"
    );
}

/// The grant door: an admin is refused on someone else's private map exactly as on a map that does
/// not exist, and on a resource homed in it; the creator grants; an admin's own map is theirs; once
/// the map is shared, the admin arm applies again.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_system_admin_does_not_grant_on_someone_elses_private_map(pool: PgPool) {
    let owner = person(&pool, "pg-owner").await;
    let admin = admin(&pool, "pg-admin").await;
    let admin_team = shared_team(&pool, "pg-admin-team", admin).await;
    let map = genesis(&pool, owner, "Private").await;
    let own_map = genesis(&pool, admin, "Admin's own").await;
    let as_owner = test_support::human_principal_for(&pool, owner).await;
    let as_admin = test_support::human_principal_for(&pool, admin).await;

    let refused = access_service::grant_capability(&pool, &as_admin, &map_grant(map, admin_team))
        .await
        .expect_err("an admin does not grant on someone else's private map");
    assert_not_found(&refused, access_service::GRANT_SUBJECT_REFUSAL);
    let missing =
        access_service::grant_capability(&pool, &as_admin, &map_grant(Uuid::now_v7(), admin_team))
            .await
            .expect_err("nor on a map that does not exist");
    assert_not_found(&missing, access_service::GRANT_SUBJECT_REFUSAL);
    let refused = access_service::revoke_capability(
        &pool,
        &as_admin,
        &RevokeCapabilityRequest {
            subject_table: "kb_cogmaps".into(),
            subject_id: map,
            principal_table: "kb_profiles".into(),
            principal_id: owner,
        },
    )
    .await
    .expect_err("nor revokes the creator's own grant");
    assert_not_found(&refused, access_service::GRANT_SUBJECT_REFUSAL);

    // The map's charter is homed in it, so it is the owner's personal space too.
    let telos: Uuid = sqlx::query_scalar("SELECT telos_resource_id FROM kb_cogmaps WHERE id = $1")
        .bind(map)
        .fetch_one(&pool)
        .await
        .unwrap();
    let resource_owner: Option<Uuid> = sqlx::query_scalar("SELECT resource_personal_owner($1)")
        .bind(telos)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(resource_owner, Some(owner));
    let refused = access_service::grant_capability(
        &pool,
        &as_admin,
        &GrantCapabilityRequest {
            subject_table: "kb_resources".into(),
            ..map_grant(telos, admin_team)
        },
    )
    .await
    .expect_err("nor on a resource homed in it");
    assert_not_found(&refused, access_service::GRANT_SUBJECT_REFUSAL);

    access_service::grant_capability(&pool, &as_admin, &map_grant(own_map, admin_team))
        .await
        .expect("an admin's own private map is theirs");

    let owner_team = shared_team(&pool, "pg-owner-team", owner).await;
    access_service::grant_capability(&pool, &as_owner, &map_grant(map, owner_team))
        .await
        .expect("the creator grants on their own map");
    // An admin's audit of the owner's acts does not show that grant: it is in their personal
    // space. The owner reads it.
    let grant_rows = |entries: &[admin_ledger_service::AdminLedgerEntry]| {
        entries
            .iter()
            .filter(|e| e.event_type == "grant_created")
            .count()
    };
    let audit =
        admin_ledger_service::list_by_actor(&pool, &as_admin, ProfileId::from(owner), 50, 0)
            .await
            .expect("an admin audits the owner's acts");
    assert_eq!(grant_rows(&audit), 0, "{audit:?}");
    let own = admin_ledger_service::list_by_actor(&pool, &as_owner, ProfileId::from(owner), 50, 0)
        .await
        .expect("the owner reads their own history");
    assert_eq!(grant_rows(&own), 1);
    cogmap_service::bind_team(
        &pool,
        &as_owner,
        map,
        &BindTeamRequest {
            team_id: owner_team,
        },
    )
    .await
    .expect("the creator shares the map with a shared team they manage");
    access_service::grant_capability(&pool, &as_admin, &map_grant(map, admin_team))
        .await
        .expect("a shared map is not personal space: the admin arm applies");
}

/// Bind and unbind: an admin is refused on someone else's private map as on a missing map.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_system_admin_does_not_bind_or_unbind_someone_elses_private_map(pool: PgPool) {
    let owner = person(&pool, "pb-owner").await;
    let admin = admin(&pool, "pb-admin").await;
    let admin_team = shared_team(&pool, "pb-admin-team", admin).await;
    let map = genesis(&pool, owner, "Private").await;
    let as_admin = test_support::human_principal_for(&pool, admin).await;
    let bind = BindTeamRequest {
        team_id: admin_team,
    };

    let refused = cogmap_service::bind_team(&pool, &as_admin, map, &bind)
        .await
        .expect_err("an admin does not bind someone else's private map");
    assert_not_found(&refused, cogmap_service::COGMAP_MISSING);
    let missing = cogmap_service::bind_team(&pool, &as_admin, Uuid::now_v7(), &bind)
        .await
        .expect_err("binding a missing map is refused");
    assert_not_found(&missing, cogmap_service::COGMAP_MISSING);
    let refused = cogmap_service::unbind_team(&pool, &as_admin, map, admin_team)
        .await
        .expect_err("nor unbinds it");
    assert_not_found(&refused, cogmap_service::COGMAP_MISSING);
    let bindings: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_team_cogmaps WHERE cogmap_id = $1")
            .bind(map)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(bindings, 0);

    // Aimed at a personal team (the admin's own), the gate refuses on the team side for any map;
    // a missing map and someone else's private map are still answered alike.
    let own_personal = BindTeamRequest {
        team_id: personal_team(&pool, admin).await,
    };
    for target in [map, Uuid::now_v7()] {
        let refused = cogmap_service::bind_team(&pool, &as_admin, target, &own_personal)
            .await
            .expect_err("refused");
        assert_not_found(&refused, cogmap_service::COGMAP_MISSING);
    }

    // Genesis at the private map's id tells the admin the id is taken, and nothing else.
    let Err(err) = DbBackend::new(pool.clone(), ProfileId::from(admin))
        .create_cognitive_map(CreateCognitiveMap {
            request: temper_core::types::reconcile::CreateCogmapRequest {
                cogmap_id: Some(map),
                telos_resource_id: None,
                name: "probe".to_string(),
                telos_title: "probe".to_string(),
                telos: None,
            },
            origin: Surface::ApiHttp,
        })
        .await
    else {
        panic!("genesis at an unreadable map's id is refused");
    };
    assert!(
        matches!(err, temper_core::error::TemperError::Conflict(_)),
        "{err:?}"
    );

    let own_map = genesis(&pool, admin, "Admin's own").await;
    cogmap_service::bind_team(&pool, &as_admin, own_map, &bind)
        .await
        .expect("an admin binds their own private map");
}

/// A person's personal team takes only that person's own map, whoever binds it; a map with no
/// person creator takes one person's personal team at most.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_map_is_bound_to_one_persons_personal_team_at_most(pool: PgPool) {
    let owner = person(&pool, "po-owner").await;
    let other = person(&pool, "po-other").await;
    let owner_personal = personal_team(&pool, owner).await;
    let other_personal = personal_team(&pool, other).await;
    let map = genesis(&pool, owner, "Private").await;
    let as_owner = test_support::human_principal_for(&pool, owner).await;
    let as_other = test_support::human_principal_for(&pool, other).await;

    cogmap_service::bind_team(
        &pool,
        &as_owner,
        map,
        &BindTeamRequest {
            team_id: owner_personal,
        },
    )
    .await
    .expect("the creator binds their map to their own personal team");
    assert_eq!(personal_owner(&pool, map).await, Some(owner));

    // The creator hands `other` grant authority; `other` manages their own personal team, so the
    // gate admits them, and the one-person rule refuses.
    access_service::grant_capability(
        &pool,
        &as_owner,
        &GrantCapabilityRequest {
            principal_table: "kb_profiles".into(),
            principal_id: other,
            can_grant: true,
            ..map_grant(map, other_personal)
        },
    )
    .await
    .expect("the creator grants to another person");
    let refused = cogmap_service::bind_team(
        &pool,
        &as_other,
        map,
        &BindTeamRequest {
            team_id: other_personal,
        },
    )
    .await
    .expect_err("another person's personal team does not take the creator's map");
    match refused {
        ApiError::ForbiddenDetail(message) => {
            assert_eq!(message, cogmap_service::PERSONAL_BINDING_REFUSAL)
        }
        other => panic!("expected the one-person refusal, got {other:?}"),
    }

    // A map with no person creator (the system's, or one from before the creator was recorded).
    let orphan = genesis(&pool, owner, "No creator").await;
    sqlx::query("UPDATE kb_cogmaps SET created_by_profile_id = NULL WHERE id = $1")
        .bind(orphan)
        .execute(&pool)
        .await
        .unwrap();
    bind_raw(&pool, orphan, other_personal)
        .await
        .expect("one person's personal team");
    assert_eq!(personal_owner(&pool, orphan).await, Some(other));
    let err = bind_raw(&pool, orphan, owner_personal)
        .await
        .expect_err("a second person's personal team is refused");
    assert!(is_tc001(&err), "{err:?}");
}

/// Share and reassign: an admin refused on someone else's personal context is told what a missing
/// context would tell them.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_refusal_on_someone_elses_personal_context_is_a_missing_context(pool: PgPool) {
    let owner = person(&pool, "px-owner").await;
    let admin = admin(&pool, "px-admin").await;
    let admin_team = shared_team(&pool, "px-admin-team", admin).await;
    let context: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, 'journal', 'journal') RETURNING id",
    )
    .bind(owner)
    .fetch_one(&pool)
    .await
    .unwrap();
    let as_admin = test_support::human_principal_for(&pool, admin).await;
    let share = ShareContextRequest {
        team_id: admin_team,
    };

    for target in [context, Uuid::now_v7()] {
        let refused = context_service::share(&pool, &as_admin, target, &share)
            .await
            .expect_err("share is refused");
        assert_not_found(&refused, context_service::CONTEXT_MISSING);
        let refused = context_service::reassign(&pool, &as_admin, target, admin_team)
            .await
            .expect_err("reassign is refused");
        assert_not_found(&refused, context_service::CONTEXT_MISSING);
    }
}
