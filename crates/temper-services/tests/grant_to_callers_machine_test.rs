#![cfg(feature = "test-db")]

//! A machine a delegate registered is held to one rule at both grant-writing doors
//! (`access_service::callers_machine_grant_bar`; v0.6.3 review S3, task
//! `01a19370-dc05-76c1-bdf3-ea2280a4be6c`, ruled 2026-10-31).
//!
//! A grant to the caller's own machine is admitted only when the caller holds the grant right on the
//! subject in their own name. That is a grant row on their own profile (a map's creator holds one
//! from genesis) or ownership of the resource's home. A grant right that reaches them only through a
//! team is refused with the self-grant's `Forbidden`, because the machine's row would outlive their
//! membership. The rule applies at the grant door (`grant_capability`) and the registration door
//! (`machine_registration_service::issue`). Every refusal here has a live control beside it that
//! still succeeds.
//!
//! ```text
//!   max:   maintainer on umbrella; umbrella holds read+write+grant on the resource and on `team_map`
//!          (max's right there is inherited through a team)
//!   max:   holds read+write+grant on `own_map` in his own name (a creator's genesis row)
//!   owner: owns the resource's home (holds its grant right in their own name)
//! ```

use sqlx::PgPool;
use temper_core::types::cognitive_maps::GrantCapabilityRequest;
use temper_core::types::machine::{GrantSpec, IssueMachineRequest};
use temper_services::auth_config::AuthMode;
use temper_services::error::ApiError;
use temper_services::services::{access_service, machine_registration_service};
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

/// A machine profile whose client `registrar` registered.
async fn machine(pool: &PgPool, handle: &str, registrar: Uuid) -> Uuid {
    let id = person(pool, handle).await;
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, 'test', $2, $3)",
    )
    .bind(format!("client-{handle}"))
    .bind(id)
    .bind(registrar)
    .execute(pool)
    .await
    .expect("seed machine client");
    id
}

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) \
         ON CONFLICT (slug) DO UPDATE SET slug = EXCLUDED.slug RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("seed team")
}

async fn cogmap(pool: &PgPool, name: &str) -> Uuid {
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $1) RETURNING id",
    )
    .bind(format!("{name}-telos"))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A read+write+grant row on `subject` to `(principal_table, principal_id)`.
async fn full_grant(
    pool: &PgPool,
    subject_table: &str,
    subject: Uuid,
    principal_table: &str,
    principal_id: Uuid,
    granted_by: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_access_grants (id, subject_table, subject_id, principal_table, principal_id, \
           can_read, can_write, can_grant, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), $1, $2, $3, $4, true, true, true, $5)",
    )
    .bind(subject_table)
    .bind(subject)
    .bind(principal_table)
    .bind(principal_id)
    .bind(granted_by)
    .execute(pool)
    .await
    .unwrap();
}

async fn rows(pool: &PgPool, subject: Uuid, principal: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants WHERE subject_id = $1 AND principal_id = $2",
    )
    .bind(subject)
    .bind(principal)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn machines_registered_by(pool: &PgPool, registrar: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_machine_clients WHERE registered_by_profile_id = $1",
    )
    .bind(registrar)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn grant(subject_table: &str, subject: Uuid, principal_id: Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: subject_table.into(),
        subject_id: subject,
        principal_table: "kb_profiles".into(),
        principal_id,
        can_read: true,
        can_write: true,
        can_delete: false,
        can_grant: false,
    }
}

struct Fixture {
    owner: Uuid,
    max: Uuid,
    max_personal: Uuid,
    resource: Uuid,
    team_map: Uuid,
    own_map: Uuid,
}

async fn fixture(pool: &PgPool) -> Fixture {
    // The root exists before profiles, so each personal team is linked under it.
    team(pool, "temper-system").await;
    let owner = person(pool, "cm-owner").await;
    let max = person(pool, "cm-max").await;

    let umbrella = team(pool, "cm-umbrella").await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'maintainer'::team_role)",
    )
    .bind(umbrella)
    .bind(max)
    .execute(pool)
    .await
    .unwrap();
    let max_personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(max)
        .fetch_one(pool)
        .await
        .unwrap();

    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, 'cm-ctx', 'cm-ctx') RETURNING id",
    )
    .bind(owner)
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
    full_grant(pool, "kb_resources", resource, "kb_teams", umbrella, owner).await;

    let team_map = cogmap(pool, "cm-team-map").await;
    full_grant(pool, "kb_cogmaps", team_map, "kb_teams", umbrella, owner).await;
    let own_map = cogmap(pool, "cm-own-map").await;
    full_grant(pool, "kb_cogmaps", own_map, "kb_profiles", max, max).await;

    Fixture {
        owner,
        max,
        max_personal,
        resource,
        team_map,
        own_map,
    }
}

/// The grant door: a grant right max holds through umbrella cannot reach his own machine. The same
/// grant to another person's machine still lands, so the refusal is about the recipient.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_inherited_grant_right_cannot_reach_the_callers_own_machine(pool: PgPool) {
    let f = fixture(&pool).await;
    let maxs_machine = machine(&pool, "cm-max-bot", f.max).await;
    let owners_machine = machine(&pool, "cm-owner-bot", f.owner).await;
    let m = test_support::human_principal_for(&pool, f.max).await;

    for (table, subject) in [("kb_resources", f.resource), ("kb_cogmaps", f.team_map)] {
        let refusal =
            access_service::grant_capability(&pool, &m, &grant(table, subject, maxs_machine))
                .await
                .expect_err("an inherited grant right cannot be given to the caller's own machine");
        assert!(
            matches!(refusal, ApiError::Forbidden),
            "{table}: {refusal:?}"
        );
        assert_eq!(
            rows(&pool, subject, maxs_machine).await,
            0,
            "{table}: nothing written"
        );

        // Live control: the same grant to someone else's machine.
        access_service::grant_capability(&pool, &m, &grant(table, subject, owners_machine))
            .await
            .unwrap_or_else(|e| panic!("{table}: a grant to another person's machine: {e:?}"));
        assert_eq!(rows(&pool, subject, owners_machine).await, 1);
    }

    // A revoked client still names its registrar: the profile is still max's machine.
    sqlx::query("UPDATE kb_machine_clients SET revoked_at = now(), revoked_by_profile_id = $1 WHERE profile_id = $2")
        .bind(f.max)
        .bind(maxs_machine)
        .execute(&pool)
        .await
        .unwrap();
    let refusal = access_service::grant_capability(
        &pool,
        &m,
        &grant("kb_resources", f.resource, maxs_machine),
    )
    .await
    .expect_err("a revoked client's profile is still the caller's machine");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
}

/// The grant door, the admitted side: a grant right held in the caller's own name may reach their
/// machine, whether it comes from a row on their profile (a map's creator) or from owning the
/// resource's home. A system admin's grant to anyone's machine is unchanged.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_grant_right_held_in_person_may_reach_the_callers_own_machine(pool: PgPool) {
    let f = fixture(&pool).await;
    let maxs_machine = machine(&pool, "cm-max-bot", f.max).await;
    let owners_machine = machine(&pool, "cm-owner-bot", f.owner).await;

    let m = test_support::human_principal_for(&pool, f.max).await;
    access_service::grant_capability(&pool, &m, &grant("kb_cogmaps", f.own_map, maxs_machine))
        .await
        .expect("a map's creator gives their own machine reach onto it");
    assert_eq!(rows(&pool, f.own_map, maxs_machine).await, 1);

    let o = test_support::human_principal_for(&pool, f.owner).await;
    access_service::grant_capability(
        &pool,
        &o,
        &grant("kb_resources", f.resource, owners_machine),
    )
    .await
    .expect("a resource's home owner gives their own machine reach onto it");
    assert_eq!(rows(&pool, f.resource, owners_machine).await, 1);

    let admin = person(&pool, "cm-admin").await;
    test_support::approved_admin(&pool, admin).await;
    let a = test_support::human_principal_for(&pool, admin).await;
    let admins_machine = machine(&pool, "cm-admin-bot", admin).await;
    for target in [maxs_machine, admins_machine] {
        access_service::grant_capability(&pool, &a, &grant("kb_cogmaps", f.team_map, target))
            .await
            .expect("a system admin's grant to a machine is unchanged");
    }
}

/// The registration door: the same rule, through `issue`. A reach grant on a map max reaches only
/// through umbrella is refused and nothing is minted. A reach grant on the map he holds in his own
/// name is admitted.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn registration_holds_reach_grants_to_the_same_rule(pool: PgPool) {
    let f = fixture(&pool).await;
    let m = test_support::human_principal_for(&pool, f.max).await;
    let req = |cogmap_id| IssueMachineRequest {
        label: "cm-bot".into(),
        owner_team_id: Some(f.max_personal),
        teams: vec![],
        grants: vec![GrantSpec {
            cogmap_id,
            can_write: true,
        }],
    };

    let refusal =
        machine_registration_service::issue(&pool, &m, &req(f.team_map), AuthMode::TemperAs)
            .await
            .expect_err("a reach grant on an inherited grant right is refused at registration");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert_eq!(
        machines_registered_by(&pool, f.max).await,
        0,
        "nothing minted"
    );

    let issued =
        machine_registration_service::issue(&pool, &m, &req(f.own_map), AuthMode::TemperAs)
            .await
            .expect("a map's creator registers a machine with reach onto it");
    assert_eq!(rows(&pool, f.own_map, issued.client.profile_id).await, 1);
}
