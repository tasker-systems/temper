#![cfg(feature = "test-db")]
//! Task 15 — demotion-by-transition and the manual `demote` act (D10 / spec §9).
//!
//! §9: "Revoke and Deactivate demote, so 'admin, but admission revoked' is never representable."
//! The invariant is maintained BY TRANSITION, and since 20261205000010 also at the read:
//! `is_system_admin` asks governance AND live standing, so a governance row left behind by a missed
//! demotion confers nothing. Both halves are pinned below.
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
    temper_services::test_support::profile::insert(pool, handle).await
}

/// An approved system admin — able both to act on the surface and to be acted upon.
async fn an_admin(pool: &PgPool, handle: &str) -> uuid::Uuid {
    let id = a_profile(pool, handle).await;
    test_support::approved_admin(pool, id).await;
    id
}

/// Mint the `SystemAdmin` proof for an admin profile — the capability the migrated acts require.
async fn admin_proof(pool: &PgPool, admin_id: uuid::Uuid) -> SystemAdmin {
    let a = test_support::human_principal_for(pool, admin_id).await;
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

    let authed = test_support::human_principal_for(&pool, non_admin).await;
    let err = require_system_admin(&pool, &authed)
        .await
        .expect_err("a non-admin may not mint an admin proof");
    assert!(matches!(err, ApiError::Forbidden));
    assert!(
        is_admin(&pool, subject).await,
        "the refused caller wrote nothing"
    );
}

// ---------------------------------------------------------------------------------------------
// The read refuses as well (20261205000010). The tests above pin the transition: Revoke and
// Deactivate delete the governance row. These pin the read: a governance row that outlives the
// principal's standing (a missed demotion, a direct write) confers nothing. Each builds that state
// directly, behind the transition's back, because no door is known to produce it.
// ---------------------------------------------------------------------------------------------

/// Move `p`'s standing without the committer, so the governance row is left behind.
async fn force_standing(pool: &PgPool, p: uuid::Uuid, state: &str) {
    sqlx::query(
        "UPDATE kb_principal_standing SET state = $2, updated = now() WHERE profile_id = $1",
    )
    .bind(p)
    .bind(state)
    .execute(pool)
    .await
    .unwrap();
}

async fn holds_governance(pool: &PgPool, p: uuid::Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_principal_governance WHERE profile_id = $1)",
    )
    .bind(p)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The predicate as a plpgsql `IF NOT` guard reads it: a NULL there falls open, so this asks
/// whether the guard fires, not what the scalar is.
async fn plpgsql_guard_refuses(pool: &PgPool, p: uuid::Uuid) -> bool {
    let body = format!(
        "DO $$ BEGIN IF NOT is_system_admin('{p}'::uuid) THEN \
         RAISE EXCEPTION 'guard fired'; END IF; END $$"
    );
    sqlx::raw_sql(&body).execute(pool).await.is_err()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn governance_without_live_standing_is_not_admin(pool: PgPool) {
    let subject = an_admin(&pool, "governs-on-paper").await;
    assert!(
        is_admin(&pool, subject).await,
        "the fixture starts as an admin"
    );

    for state in ["revoked", "deactivated", "denied", "requested"] {
        force_standing(&pool, subject, state).await;
        assert!(
            holds_governance(&pool, subject).await,
            "the row is left behind ({state})"
        );

        let scalar: Option<bool> = sqlx::query_scalar("SELECT is_system_admin($1)")
            .bind(subject)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            scalar,
            Some(false),
            "standing {state}: never admin, and never NULL"
        );
        assert!(
            !is_admin(&pool, subject).await,
            "standing {state}: the Rust read refuses"
        );
        assert!(
            plpgsql_guard_refuses(&pool, subject).await,
            "standing {state}: an `IF NOT is_system_admin` guard fires"
        );
    }

    // The same row with live standing governs again: the read follows standing, not the row alone.
    force_standing(&pool, subject, "approved").await;
    assert!(is_admin(&pool, subject).await);
    assert!(!plpgsql_guard_refuses(&pool, subject).await);

    // Absence: a governance row on a profile with no standing row at all (profiles are born with
    // none). Written raw, because `grant_governance` approves first.
    let unadmitted = a_profile(&pool, "governs-unadmitted").await;
    sqlx::query("INSERT INTO kb_principal_governance (profile_id) VALUES ($1)")
        .bind(unadmitted)
        .execute(&pool)
        .await
        .unwrap();
    let no_row: Option<String> = standing(&pool, unadmitted).await;
    assert_eq!(no_row, None, "the fixture has no standing row");
    let scalar: Option<bool> = sqlx::query_scalar("SELECT is_system_admin($1)")
        .bind(unadmitted)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        scalar,
        Some(false),
        "absent standing: never admin, and never NULL"
    );
    assert!(!is_admin(&pool, unadmitted).await);
    assert!(plpgsql_guard_refuses(&pool, unadmitted).await);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_revoked_admin_with_governance_is_refused_at_admin_doors(pool: PgPool) {
    let revoked = an_admin(&pool, "revoked-but-governs").await;
    let target = a_profile(&pool, "deactivation-target").await;
    test_support::approve(&pool, target).await;

    // A proof minted while live, as a session would hold one across the revocation.
    let held = admin_proof(&pool, revoked).await;
    let human = test_support::human_principal_for(&pool, revoked).await;

    force_standing(&pool, revoked, "revoked").await;
    assert!(holds_governance(&pool, revoked).await);

    // The mint every admin door runs refuses.
    assert!(matches!(
        require_system_admin(&pool, &human).await,
        Err(ApiError::Forbidden)
    ));
    // So does the locked re-check a door runs on its write transaction.
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        temper_services::auth::require_system_admin_locked(&mut tx, &human).await,
        Err(ApiError::Forbidden)
    ));
    drop(tx);
    // And an admin act driven by the held proof re-checks and is refused, writing nothing.
    let refused = access_service::admin_deactivate(&pool, &held, ProfileId::from(target)).await;
    assert!(
        matches!(refused, Err(ApiError::Forbidden)),
        "got {refused:?}"
    );
    assert_eq!(standing(&pool, target).await.as_deref(), Some("approved"));

    // With live standing the same principal, the same row and the same proof are admitted.
    force_standing(&pool, revoked, "approved").await;
    assert!(require_system_admin(&pool, &human).await.is_ok());
    access_service::admin_deactivate(&pool, &held, ProfileId::from(target))
        .await
        .unwrap();
    assert_eq!(
        standing(&pool, target).await.as_deref(),
        Some("deactivated")
    );
}
