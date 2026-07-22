#![cfg(feature = "test-db")]
//! Task 15 — demotion-by-transition and the manual `demote` act (D10 / spec §9).
//!
//! §9: "Revoke and Deactivate demote, so 'admin, but admission revoked' is never representable."
//! The invariant is maintained BY TRANSITION — `is_system_admin` reads governance and nothing else,
//! so if the demotion does not fire, a revoked admin stays admin. That is the whole risk here.
//!
//! Governance and admission are independent axes: `demote` removes the authority to change the rules
//! without touching access, and `reactivate` restores access without returning authority.
//!
//! Post-enclosure (admin-authz Task 2): the admin acts take a `&SystemAdmin` proof rather than a bare
//! `actor: ProfileId`. The F-3 gate has moved *up* into the proof — a non-admin is now refused at the
//! `require_system_admin` mint, before any act is even reachable.

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_services::auth::{require_system_admin, SystemAdmin};
use temper_services::error::ApiError;
use temper_services::services::access_service;
use temper_services::test_support;

async fn a_profile(pool: &PgPool, handle: &str) -> uuid::Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// An approved system admin — able both to act on the surface and to be acted upon.
async fn an_admin(pool: &PgPool, handle: &str) -> uuid::Uuid {
    let id = a_profile(pool, handle).await;
    test_support::approved_admin(pool, id).await;
    id
}

/// Mint the `SystemAdmin` proof for an admin profile — the capability the migrated acts require.
async fn admin_proof(pool: &PgPool, admin_id: uuid::Uuid) -> SystemAdmin {
    let a = test_support::authenticated_profile_for(pool, admin_id).await;
    require_system_admin(pool, &a)
        .await
        .expect("admin mints a proof")
}

async fn is_admin(pool: &PgPool, p: uuid::Uuid) -> bool {
    access_service::is_system_admin(pool, ProfileId::from(p))
        .await
        .unwrap()
}

async fn standing(pool: &PgPool, p: uuid::Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id = $1")
        .bind(p)
        .fetch_optional(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revoke_demotes_so_admin_but_revoked_is_never_representable(pool: PgPool) {
    let actor = an_admin(&pool, "revoker").await;
    let subject = an_admin(&pool, "soon-revoked").await;
    assert!(is_admin(&pool, subject).await);

    let proof = admin_proof(&pool, actor).await;
    access_service::admin_revoke(&pool, &proof, ProfileId::from(subject), "test".into())
        .await
        .unwrap();

    assert!(
        !is_admin(&pool, subject).await,
        "'admin, but admission revoked' must never be representable (§9)"
    );
    assert_eq!(standing(&pool, subject).await.as_deref(), Some("revoked"));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn deactivate_demotes_too(pool: PgPool) {
    let actor = an_admin(&pool, "deactivator").await;
    let subject = an_admin(&pool, "soon-deactivated").await;
    assert!(is_admin(&pool, subject).await);

    let proof = admin_proof(&pool, actor).await;
    access_service::admin_deactivate(&pool, &proof, ProfileId::from(subject))
        .await
        .unwrap();

    assert!(!is_admin(&pool, subject).await);
    assert_eq!(
        standing(&pool, subject).await.as_deref(),
        Some("deactivated")
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reactivating_a_demoted_admin_does_not_restore_governance(pool: PgPool) {
    // Reactivate restores STANDING (§5). It says nothing about governance; silently re-admining on
    // reactivation would make a deactivation a round-trip that quietly returns authority.
    let actor = an_admin(&pool, "reactivator").await;
    let subject = an_admin(&pool, "round-trip").await;

    let proof = admin_proof(&pool, actor).await;
    access_service::admin_deactivate(&pool, &proof, ProfileId::from(subject))
        .await
        .unwrap();
    access_service::admin_reactivate(&pool, &proof, ProfileId::from(subject))
        .await
        .unwrap();

    assert_eq!(
        standing(&pool, subject).await.as_deref(),
        Some("approved"),
        "standing is restored"
    );
    assert!(
        !is_admin(&pool, subject).await,
        "governance is NOT restored — re-promotion is its own act"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn demote_admin_revokes_governance_but_leaves_standing(pool: PgPool) {
    // The manual governance twin of `promote`. Governance and admission are independent axes: a
    // demoted admin keeps its access, it just may no longer change the rules (D10).
    let actor = an_admin(&pool, "demoter").await;
    let subject = an_admin(&pool, "demoted").await;
    assert!(is_admin(&pool, subject).await);

    let proof = admin_proof(&pool, actor).await;
    access_service::demote_admin(&pool, &proof, ProfileId::from(subject))
        .await
        .unwrap();

    assert!(!is_admin(&pool, subject).await);
    assert_eq!(
        standing(&pool, subject).await.as_deref(),
        Some("approved"),
        "demotion is governance-only; access is untouched"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn demote_admin_requires_the_caller_be_admin(pool: PgPool) {
    // F-3, now typed: the gate is the proof. A non-admin caller cannot mint a `SystemAdmin`, so the
    // refusal happens BEFORE any act is reachable — `demote_admin` never runs without one.
    let subject = an_admin(&pool, "still-admin").await;
    let non_admin = a_profile(&pool, "not-admin").await;
    test_support::approve(&pool, non_admin).await; // has access, not governance

    let authed = test_support::authenticated_profile_for(&pool, non_admin).await;
    let err = require_system_admin(&pool, &authed)
        .await
        .expect_err("a non-admin may not mint an admin proof");
    assert!(matches!(err, ApiError::Forbidden));
    assert!(
        is_admin(&pool, subject).await,
        "the refused caller wrote nothing"
    );
}
