#![cfg(feature = "test-db")]

//! A system admin's grant administration on a retired context: **grant refused, revoke admitted**
//! (ruled 2026-10-17).
//!
//! The delegated arm already answers a retired context "no" through `can()`'s subject-liveness
//! floor (migration `20260902000010`). The admin arm's floor for a context is grant-only and lives
//! in `access_service::authorize_capability_grant`, so revocation, which resolves the same
//! `GrantAuthority` without that function, keeps working: an admin can still withdraw a grant
//! from a retired context, and the context stays retired.
//!
//! The fixture is inline, as everywhere in `crates/temper-services/tests/`.

use sqlx::PgPool;
use temper_core::types::cognitive_maps::{GrantCapabilityRequest, RevokeCapabilityRequest};
use temper_services::error::ApiError;
use temper_services::services::{access_service, context_service};
use temper_services::test_support;
use temper_workflow::operations::Surface;
use uuid::Uuid;

/// A profile with its per-surface emitter entities: `resolve_emitter` creates none lazily, so a
/// profile without them cannot author a grant, a revoke or a retirement.
async fn profile_with_emitters(pool: &PgPool, handle: &str) -> Uuid {
    let profile_id: Uuid = sqlx::query_scalar(
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
        .bind(profile_id)
        .bind(format!("{handle}@{}", surface.marker()))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    profile_id
}

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("seed team")
}

async fn team_context(pool: &PgPool, team_id: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(team_id)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("seed context")
}

fn grant_req(context: Uuid, team: Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_contexts".into(),
        subject_id: context,
        principal_table: "kb_teams".into(),
        principal_id: team,
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
    }
}

fn revoke_req(context: Uuid, team: Uuid) -> RevokeCapabilityRequest {
    RevokeCapabilityRequest {
        subject_table: "kb_contexts".into(),
        subject_id: context,
        principal_table: "kb_teams".into(),
        principal_id: team,
    }
}

async fn grant_rows(pool: &PgPool, context: Uuid, team: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants \
          WHERE subject_table = 'kb_contexts' AND subject_id = $1 \
            AND principal_table = 'kb_teams' AND principal_id = $2",
    )
    .bind(context)
    .bind(team)
    .fetch_one(pool)
    .await
    .expect("count grant rows")
}

async fn context_state(pool: &PgPool, context: Uuid) -> (bool, String) {
    sqlx::query_as("SELECT is_active, slug FROM kb_contexts WHERE id = $1")
        .bind(context)
        .fetch_one(pool)
        .await
        .expect("read context")
}

/// FAILS IF a system admin can grant on a retired context, or can no longer revoke there, or the
/// revoke touches the context. The bite: in `authorize_capability_grant`, drop the context check
/// from the `SystemAdmin` arm, and the grant on the retired context returns `Ok`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_system_admin_may_revoke_but_not_grant_on_a_retired_context(pool: PgPool) {
    let admin = profile_with_emitters(&pool, "retired-floor-admin").await;
    test_support::approved_admin(&pool, admin).await;
    assert!(
        access_service::is_system_admin(&pool, admin.into())
            .await
            .unwrap(),
        "fixture: the admin is a real system admin"
    );
    let owner_team = team(&pool, "retired-floor-owner").await;
    let holder = team(&pool, "retired-floor-holder").await;
    let later = team(&pool, "retired-floor-later").await;
    let context = team_context(&pool, owner_team, "retired-floor-ctx").await;
    let principal = test_support::human_principal_for(&pool, admin).await;

    // Live: the admin arm admits a grant on a context it has no other standing on.
    let outcome = access_service::grant_capability(&pool, &principal, &grant_req(context, holder))
        .await
        .expect("precondition: an admin grants on a live context");
    assert!(outcome.granted);
    assert_eq!(grant_rows(&pool, context, holder).await, 1);

    context_service::retire(
        &pool,
        &test_support::caller_for(&pool, admin).await,
        context,
    )
    .await
    .expect("retire the context");
    let (active, retired_slug) = context_state(&pool, context).await;
    assert!(!active, "fixture: the context is retired");

    // Retired: a grant is refused with the door's existing deny, for a new principal and for a
    // re-grant of the row that already exists, and writes nothing.
    for (team_id, label) in [(later, "a new principal"), (holder, "the existing holder")] {
        let refusal =
            access_service::grant_capability(&pool, &principal, &grant_req(context, team_id))
                .await
                .expect_err("an admin grant on a retired context is refused");
        assert!(
            matches!(refusal, ApiError::Forbidden),
            "grant to {label}: the refusal is the existing 403, got {refusal:?}"
        );
    }
    assert_eq!(grant_rows(&pool, context, later).await, 0, "no row written");
    assert_eq!(
        grant_rows(&pool, context, holder).await,
        1,
        "the existing row is untouched by the refused re-grant"
    );

    // Retired: a revoke is admitted, removes the row, and leaves the context exactly as retired.
    let outcome =
        access_service::revoke_capability(&pool, &principal, &revoke_req(context, holder))
            .await
            .expect("an admin revokes on a retired context");
    assert!(outcome.revoked, "the row existed, so the revoke reports it");
    assert_eq!(
        grant_rows(&pool, context, holder).await,
        0,
        "the row is gone"
    );
    assert_eq!(
        context_state(&pool, context).await,
        (false, retired_slug),
        "the revoke changes nothing about the retired context"
    );
}

/// An id no context row backs reads as not live, as `can()`'s floor reads it: the admin arm does
/// not mint a grant row on a subject that does not exist.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_system_admin_cannot_grant_on_a_context_that_does_not_exist(pool: PgPool) {
    let admin = profile_with_emitters(&pool, "absent-floor-admin").await;
    test_support::approved_admin(&pool, admin).await;
    // Without this the delegated arm's own refusal would pass the test for the wrong reason.
    assert!(
        access_service::is_system_admin(&pool, admin.into())
            .await
            .unwrap(),
        "fixture: the admin is a real system admin"
    );
    let holder = team(&pool, "absent-floor-holder").await;
    let absent = Uuid::now_v7();
    let principal = test_support::human_principal_for(&pool, admin).await;

    let refusal = access_service::grant_capability(&pool, &principal, &grant_req(absent, holder))
        .await
        .expect_err("an admin grant on an absent context is refused");
    // The admin's refusal on an id naming nothing is the one they get on someone else's personal
    // space (task `01a1933d`), so neither confirms what the id is.
    assert!(
        matches!(&refusal, ApiError::NotFound(m) if m == access_service::GRANT_SUBJECT_REFUSAL),
        "the refusal is the concealed 404, got {refusal:?}"
    );
    assert_eq!(grant_rows(&pool, absent, holder).await, 0, "no row written");
}
