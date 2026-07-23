#![cfg(feature = "test-db")]
//! The Level-3 `SystemAdmin` proof (spec §3.1): minted only by `require_system_admin`, which checks
//! governance. A non-admin cannot obtain one; the actor it carries is the caller.

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_services::auth::require_system_admin;
use temper_services::auth::AuthenticatedProfile;
use temper_services::error::ApiError;
use temper_services::test_support;

/// Seed a profile and build an `AuthenticatedProfile` for it — the auth path's Level-1 output.
async fn authed(pool: &PgPool, handle: &str) -> AuthenticatedProfile {
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap();
    test_support::authenticated_profile_for(pool, id).await
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn admin_gets_a_proof_carrying_its_actor(pool: PgPool) {
    let a = authed(&pool, "admin").await;
    let id = ProfileId::from(a.profile().id);
    test_support::grant_governance(&pool, a.profile().id).await;

    let proof = require_system_admin(&pool, &a)
        .await
        .expect("admin mints a proof");
    assert_eq!(proof.actor(), id, "the proof carries the acting admin");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn non_admin_is_refused(pool: PgPool) {
    let a = authed(&pool, "not-admin").await;
    let err = require_system_admin(&pool, &a)
        .await
        .expect_err("a non-admin cannot mint one");
    assert!(matches!(err, ApiError::Forbidden));
}
