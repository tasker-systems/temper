#![cfg(feature = "test-db")]

//! The grant door checks the RECIPIENT, and a delegate changes only rows that are theirs to change
//! (`access_service::{grant_capability, revoke_capability}`; temper-artifacts
//! `specs/2026-10-30-team-authority-fixes-design.md` D2).
//!
//! Authority over a subject decides what may be conferred. It never decided who receives it, so a
//! delegate could grant to any team id — `temper-system`, which every profile reaches, included —
//! grant to themselves, and overwrite or revoke anyone's row, the grant their own authority came
//! from among them.
//!
//! ```text
//!   umbrella ─▸ squad
//!   max:   maintainer on umbrella (holds can_grant through umbrella's grant: a delegate)
//!   wanda: watcher on squad       (holds nothing but read)
//!   owner: owns the resource
//! ```

use sqlx::PgPool;
use temper_core::types::cognitive_maps::{GrantCapabilityRequest, RevokeCapabilityRequest};
use temper_services::error::ApiError;
use temper_services::services::access_service;
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

async fn join(pool: &PgPool, team_id: Uuid, who: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team_id)
    .bind(who)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
}

async fn can_modify(pool: &PgPool, who: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(who)
        .bind(resource)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn rows(pool: &PgPool, resource: Uuid, principal: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants WHERE subject_id = $1 AND principal_id = $2",
    )
    .bind(resource)
    .bind(principal)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn grant(resource: Uuid, principal_table: &str, principal_id: Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_resources".into(),
        subject_id: resource,
        principal_table: principal_table.into(),
        principal_id,
        can_read: true,
        can_write: true,
        can_delete: false,
        can_grant: false,
    }
}

fn revoke(resource: Uuid, principal_table: &str, principal_id: Uuid) -> RevokeCapabilityRequest {
    RevokeCapabilityRequest {
        subject_table: "kb_resources".into(),
        subject_id: resource,
        principal_table: principal_table.into(),
        principal_id,
    }
}

struct Fixture {
    root: Uuid,
    umbrella: Uuid,
    squad: Uuid,
    unrelated: Uuid,
    owner: Uuid,
    max: Uuid,
    wanda: Uuid,
    resource: Uuid,
}

async fn fixture(pool: &PgPool) -> Fixture {
    // The root exists before profiles, so each personal team is linked under it.
    let root = team(pool, "temper-system").await;
    let owner = person(pool, "rb-owner").await;
    let max = person(pool, "rb-max").await;
    let wanda = person(pool, "rb-wanda").await;

    let umbrella = team(pool, "rb-umbrella").await;
    let squad = team(pool, "rb-squad").await;
    let unrelated = team(pool, "rb-unrelated").await;
    sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
        .bind(squad)
        .bind(umbrella)
        .execute(pool)
        .await
        .unwrap();
    join(pool, umbrella, max, "maintainer").await;
    join(pool, squad, max, "maintainer").await;
    join(pool, squad, wanda, "watcher").await;

    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, 'rb-ctx', 'rb-ctx') RETURNING id",
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
    // The owner's share: read+write+grant to the UMBRELLA.
    sqlx::query(
        "INSERT INTO kb_access_grants (id, subject_table, subject_id, principal_table, principal_id, \
           can_read, can_write, can_grant, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), 'kb_resources', $1, 'kb_teams', $2, true, true, true, $3)",
    )
    .bind(resource)
    .bind(umbrella)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();

    Fixture {
        root,
        umbrella,
        squad,
        unrelated,
        owner,
        max,
        wanda,
        resource,
    }
}

/// What used to be the laundering chain: an inherited `can_grant` used to self-grant, broadcast to
/// `temper-system` and evict its source. Every step is refused now, and nothing is written.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_inherited_grant_right_cannot_launder_broadcast_or_evict(pool: PgPool) {
    let f = fixture(&pool).await;
    let w = test_support::human_principal_for(&pool, f.wanda).await;

    for (table, id) in [("kb_profiles", f.wanda), ("kb_teams", f.root)] {
        let refusal = access_service::grant_capability(&pool, &w, &grant(f.resource, table, id))
            .await
            .expect_err("a child-team watcher holds no grant right");
        assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    }
    let refusal =
        access_service::revoke_capability(&pool, &w, &revoke(f.resource, "kb_teams", f.umbrella))
            .await
            .expect_err("nor may she withdraw the umbrella's grant");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");

    assert_eq!(rows(&pool, f.resource, f.wanda).await, 0);
    assert_eq!(rows(&pool, f.resource, f.root).await, 0);
    assert_eq!(rows(&pool, f.resource, f.umbrella).await, 1);
    let stranger = person(&pool, "rb-stranger").await;
    assert!(!can_modify(&pool, stranger, f.resource).await);
}

/// A real delegate (direct maintainer of the granted team) is bounded on the recipient.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_delegate_grants_only_to_teams_they_manage(pool: PgPool) {
    let f = fixture(&pool).await;
    let m = test_support::human_principal_for(&pool, f.max).await;

    // Admitted: a team max manages directly.
    access_service::grant_capability(&pool, &m, &grant(f.resource, "kb_teams", f.squad))
        .await
        .expect("a delegate grants to a team they manage");

    // Refused: the root, a team they do not manage, a nonexistent team, themselves.
    for (table, id, why) in [
        ("kb_teams", f.root, "temper-system"),
        ("kb_teams", f.unrelated, "a team they do not manage"),
        ("kb_teams", Uuid::now_v7(), "a team that does not exist"),
        ("kb_profiles", f.max, "themselves"),
    ] {
        let refusal = access_service::grant_capability(&pool, &m, &grant(f.resource, table, id))
            .await
            .expect_err(why);
        assert!(matches!(refusal, ApiError::Forbidden), "{why}: {refusal:?}");
    }
    // Admitted: another person.
    access_service::grant_capability(&pool, &m, &grant(f.resource, "kb_profiles", f.wanda))
        .await
        .expect("a delegate shares with another person");
}

/// A delegate overwrites and revokes only rows that are theirs; the owner and an admin revoke any.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_delegate_changes_only_their_own_rows(pool: PgPool) {
    let f = fixture(&pool).await;
    let m = test_support::human_principal_for(&pool, f.max).await;
    let o = test_support::human_principal_for(&pool, f.owner).await;

    // The owner's row on the umbrella: max manages the umbrella, so he may change it...
    // ...but a row the owner granted to a PERSON is not his.
    access_service::grant_capability(&pool, &o, &grant(f.resource, "kb_profiles", f.wanda))
        .await
        .expect("owner shares with wanda");
    let refusal =
        access_service::revoke_capability(&pool, &m, &revoke(f.resource, "kb_profiles", f.wanda))
            .await
            .expect_err("a delegate does not withdraw the owner's grant to a person");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    let refusal =
        access_service::grant_capability(&pool, &m, &grant(f.resource, "kb_profiles", f.wanda))
            .await
            .expect_err("nor overwrite it");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");

    // The owner revokes anything, including max's source grant.
    access_service::revoke_capability(&pool, &o, &revoke(f.resource, "kb_teams", f.umbrella))
        .await
        .expect("the owner withdraws any grant on their resource");
    assert_eq!(rows(&pool, f.resource, f.umbrella).await, 0);
}

/// An admin may grant to any live team, never to a soft-deleted or nonexistent one.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_admin_grants_to_any_live_team_and_no_dangling_one(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = person(&pool, "rb-admin").await;
    test_support::approved_admin(&pool, admin).await;
    let a = test_support::human_principal_for(&pool, admin).await;

    access_service::grant_capability(&pool, &a, &grant(f.resource, "kb_teams", f.unrelated))
        .await
        .expect("an admin grants to any live team");
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(f.unrelated)
        .execute(&pool)
        .await
        .unwrap();
    for id in [f.unrelated, Uuid::now_v7()] {
        let refusal =
            access_service::grant_capability(&pool, &a, &grant(f.resource, "kb_teams", id))
                .await
                .expect_err("no grant to a dead or absent team");
        assert!(matches!(refusal, ApiError::NotFound(_)), "{refusal:?}");
    }
}
