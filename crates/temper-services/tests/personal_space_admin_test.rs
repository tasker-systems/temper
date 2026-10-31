#![cfg(feature = "test-db")]

//! A system admin does not act on, or read into, someone else's personal space (task `01a191b4`):
//! not grant or revoke on a resource homed in it, not read its grant history on either ledger axis,
//! and not aim a re-embed at it. Its owner keeps every act; an admin's own personal space stays
//! theirs.
//!
//! ```text
//!   owner: @owner/journal ── resource
//!          owner-team (owner is owner), granted read on the resource by the owner
//!   admin: a system admin, owner of admin-team
//! ```

use sqlx::PgPool;
use temper_core::types::cognitive_maps::{GrantCapabilityRequest, RevokeCapabilityRequest};
use temper_core::types::ids::ProfileId;
use temper_services::error::ApiError;
use temper_services::services::embed_service::{self, ReembedScope};
use temper_services::services::{access_service, admin_ledger_service};
use temper_services::test_support;
use temper_substrate::payloads::{AnchorTable, RefTarget};
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

/// A resource homed in `owner`'s own `@` context.
async fn personal_resource(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap();
    let resource: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri) VALUES (uuid_generate_v7(), 'r', '') \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (id, resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES (uuid_generate_v7(), $1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    resource
}

fn grant(resource: Uuid, team: Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_resources".into(),
        subject_id: resource,
        principal_table: "kb_teams".into(),
        principal_id: team,
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
    }
}

fn subject(resource: Uuid) -> RefTarget {
    RefTarget {
        kind: AnchorTable::Resources,
        id: resource,
    }
}

/// How many grant records a ledger page carries.
macro_rules! grant_rows {
    ($entries:expr) => {
        $entries
            .iter()
            .filter(|e| matches!(e.event_type.as_str(), "grant_created" | "grant_revoked"))
            .count()
    };
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_system_admin_neither_grants_on_nor_audits_someone_elses_personal_space(pool: PgPool) {
    let owner = person(&pool, "ps-owner").await;
    let admin = person(&pool, "ps-admin").await;
    test_support::approved_admin(&pool, admin).await;
    let owner_team = team_owned_by(&pool, "ps-owner-team", owner).await;
    let admin_team = team_owned_by(&pool, "ps-admin-team", admin).await;
    let resource = personal_resource(&pool, owner, "ps-journal").await;

    let as_owner = test_support::human_principal_for(&pool, owner).await;
    let as_admin = test_support::human_principal_for(&pool, admin).await;

    access_service::grant_capability(&pool, &as_owner, &grant(resource, owner_team))
        .await
        .expect("the owner grants on their own personal space");

    let refused = access_service::grant_capability(&pool, &as_admin, &grant(resource, admin_team))
        .await
        .expect_err("an admin does not grant on someone else's personal space");
    assert!(matches!(refused, ApiError::Forbidden), "{refused:?}");
    let refused = access_service::revoke_capability(
        &pool,
        &as_admin,
        &RevokeCapabilityRequest {
            subject_table: "kb_resources".into(),
            subject_id: resource,
            principal_table: "kb_teams".into(),
            principal_id: owner_team,
        },
    )
    .await
    .expect_err("nor revoke there");
    assert!(matches!(refused, ApiError::Forbidden), "{refused:?}");
    let grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants WHERE subject_id = $1 AND principal_table = 'kb_teams'",
    )
    .bind(resource)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(grants, 1, "only the owner's grant stands");

    // The ledger: the owner reads the grant on the subject axis; the admin reads it on neither.
    let owners = admin_ledger_service::list_by_subject(&pool, &as_owner, subject(resource), 50, 0)
        .await
        .expect("the owner reads their own subject");
    assert_eq!(grant_rows!(owners), 1);
    // Either no readable act family (the axis answers 404) or a page without the grant acts; any
    // other error is a failure, not an empty page.
    match admin_ledger_service::list_by_subject(&pool, &as_admin, subject(resource), 50, 0).await {
        Ok(admins) => assert_eq!(grant_rows!(admins), 0, "{admins:?}"),
        Err(ApiError::NotFound(_)) => {}
        Err(other) => panic!("the admin's subject read failed: {other:?}"),
    }
    let audit =
        admin_ledger_service::list_by_actor(&pool, &as_admin, ProfileId::from(owner), 50, 0)
            .await
            .expect("an admin audits the owner's acts");
    assert_eq!(grant_rows!(audit), 0, "{audit:?}");
    let own = admin_ledger_service::list_by_actor(&pool, &as_owner, ProfileId::from(owner), 50, 0)
        .await
        .expect("the owner reads their own history");
    assert_eq!(grant_rows!(own), 1);
}

/// An admin's own personal space stays theirs: they grant on it like any owner, and read its
/// history.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_system_admins_own_personal_space_is_theirs(pool: PgPool) {
    let admin = person(&pool, "ps-admin").await;
    test_support::approved_admin(&pool, admin).await;
    let admin_team = team_owned_by(&pool, "ps-admin-team", admin).await;
    let resource = personal_resource(&pool, admin, "ps-own").await;
    let as_admin = test_support::human_principal_for(&pool, admin).await;

    access_service::grant_capability(&pool, &as_admin, &grant(resource, admin_team))
        .await
        .expect("an admin grants on their own personal space");
    let history = admin_ledger_service::list_by_subject(&pool, &as_admin, subject(resource), 50, 0)
        .await
        .unwrap();
    assert_eq!(grant_rows!(history), 1);
}

/// A re-embed does not aim at someone else's personal space, and its sweep's receipt does not name
/// it; the operator's own personal space is theirs.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_reembed_does_not_aim_at_or_name_someone_elses_personal_space(pool: PgPool) {
    let owner = person(&pool, "ps-owner").await;
    let admin = person(&pool, "ps-admin").await;
    test_support::approved_admin(&pool, admin).await;
    let proof = test_support::system_admin_proof_for(&pool, admin).await;
    let theirs = personal_resource(&pool, owner, "ps-journal").await;
    let mine = personal_resource(&pool, admin, "ps-own").await;

    assert!(
        embed_service::scope_is_personal(&pool, &proof, ReembedScope::Resource(theirs))
            .await
            .unwrap()
    );
    assert!(
        !embed_service::scope_is_personal(&pool, &proof, ReembedScope::Resource(mine))
            .await
            .unwrap()
    );
    assert!(
        !embed_service::scope_is_personal(&pool, &proof, ReembedScope::All)
            .await
            .unwrap()
    );
    let named = embed_service::unnamed_personal(&pool, &proof, vec![theirs, mine])
        .await
        .unwrap();
    assert_eq!(named, vec![mine]);
}
