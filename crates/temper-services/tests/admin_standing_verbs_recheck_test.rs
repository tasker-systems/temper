#![cfg(feature = "test-db")]
//! The admin standing and governance acts ask the admin question again on their own transaction,
//! before any write, with the locked governance read (`auth::is_system_admin_locked`). Their
//! `&SystemAdmin` proof is minted on the pool, so before this an admin demoted after the mint, or
//! while the act was in flight, still approved, revoked, deactivated, reactivated, promoted (their
//! own demotion included: a promote's governance insert waited for the delete, then inserted),
//! demoted, reconciled the auto-join pools and closed reviews.
//!
//! Each verb is raced twice. **In flight:** the actor's demotion runs in a transaction left open,
//! the verb is started, the test waits until the verb is blocked on the demotion's backend, and the
//! demotion commits; the verb is refused (403) and wrote nothing. **Committed first:** the proof is
//! minted, the demotion commits, then the verb is called with the stale proof; it is refused. Both
//! halves fail without the re-check (no wait, and an admin act written).
//!
//! Lock order. A standing act takes the subject's standing row first, then governance. The acts
//! that delete the subject's governance row (Revoke, Deactivate, demote) lock the subject's and the
//! actor's governance rows together in ascending profile id order: two admins demoting each other
//! at once otherwise each hold their own row `FOR KEY SHARE` and wait on the other's. The team and
//! invitation doors take governance after their own rows and take no standing row, and a demotion
//! takes no row of theirs, so no pairing waits in a cycle. The last tests race mutual demotions and
//! mutual revokes.

use std::future::Future;

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_core::types::machine::ProvisionMachineRequest;
use temper_principal::Provisioner;
use temper_services::auth::SystemAdmin;
use temper_services::auth_config::AuthMode;
use temper_services::error::ApiError;
use temper_services::services::{
    access_service, machine_client_service, machine_registration_service, standing_service,
};
use temper_services::test_support;
use temper_workflow::operations::Surface;
use uuid::Uuid;

// ── Fixture ───────────────────────────────────────────────────────────────────────────────────────

async fn profile(pool: &PgPool, handle: &str) -> Uuid {
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
    id
}

async fn an_admin(pool: &PgPool, handle: &str) -> Uuid {
    let id = profile(pool, handle).await;
    test_support::approved_admin(pool, id).await;
    id
}

async fn approved(pool: &PgPool, handle: &str) -> Uuid {
    let id = profile(pool, handle).await;
    test_support::approve(pool, id).await;
    id
}

/// Born `Denied`: the state a direct approval admits from.
async fn denied(pool: &PgPool, handle: &str) -> Uuid {
    let id = profile(pool, handle).await;
    standing_service::provision(pool, ProfileId::from(id), Provisioner::OauthFirstLogin)
        .await
        .expect("born denied");
    id
}

async fn proof(pool: &PgPool, admin: Uuid) -> SystemAdmin {
    test_support::system_admin_proof_for(pool, admin).await
}

async fn standing(pool: &PgPool, p: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id = $1")
        .bind(p)
        .fetch_optional(pool)
        .await
        .unwrap()
}

async fn governs(pool: &PgPool, p: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_principal_governance WHERE profile_id = $1)",
    )
    .bind(p)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("team")
}

// ── The two races ─────────────────────────────────────────────────────────────────────────────────

async fn backend_pid(conn: &mut sqlx::PgConnection) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .expect("pid")
}

/// Wait until some backend in this database is blocked on a lock `holder` holds.
async fn await_blocked_by(pool: &PgPool, holder: i32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)))",
        )
        .bind(holder)
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the verb never blocked on the demotion: it did not re-check under its locks"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// `who`'s demotion in a transaction left open: the governance row is deleted and locked until the
/// holder commits. `principal_governance_set` is what `demote_admin` and `standing_service::apply`
/// (on Revoke and Deactivate) both run.
async fn demotion_in_flight(
    pool: &PgPool,
    who: Uuid,
) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    let _: bool = sqlx::query_scalar("SELECT principal_governance_set($1, false, NULL, 'test')")
        .bind(who)
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    (holder, pid)
}

/// The verb, started with a proof minted before `admin`'s demotion while the demotion is in
/// flight, waits on it and is refused once it commits. Then the verb is called again with that
/// stale proof after the demotion committed, and is refused. `admin` is an admin again afterwards.
async fn both_races<T, F, Fut>(pool: &PgPool, admin: Uuid, verb: F)
where
    T: std::fmt::Debug + Send + 'static,
    F: Fn(PgPool, SystemAdmin) -> Fut,
    Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    let stale = proof(pool, admin).await;
    let (holder, pid) = demotion_in_flight(pool, admin).await;
    let contender = tokio::spawn(verb(pool.clone(), stale));
    await_blocked_by(pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "in flight: {err:?}");
    test_support::grant_governance(pool, admin).await;

    let stale = proof(pool, admin).await;
    let (holder, _) = demotion_in_flight(pool, admin).await;
    holder.commit().await.unwrap();
    let err = verb(pool.clone(), stale).await.unwrap_err();
    assert!(
        matches!(err, ApiError::Forbidden),
        "committed first: {err:?}"
    );
    test_support::grant_governance(pool, admin).await;
}

// ── The verbs ─────────────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn approve(pool: PgPool) {
    let admin = an_admin(&pool, "sv-admin").await;
    let subject = denied(&pool, "sv-subject").await;
    let verb = move |pool: PgPool, proof: SystemAdmin| async move {
        access_service::admin_approve(&pool, &proof, ProfileId::from(subject)).await
    };
    both_races(&pool, admin, &verb).await;
    assert_eq!(standing(&pool, subject).await.as_deref(), Some("denied"));
    verb(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin approves");
    assert_eq!(standing(&pool, subject).await.as_deref(), Some("approved"));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revoke(pool: PgPool) {
    let admin = an_admin(&pool, "sv-admin").await;
    let subject = approved(&pool, "sv-subject").await;
    let verb = move |pool: PgPool, proof: SystemAdmin| async move {
        access_service::admin_revoke(&pool, &proof, ProfileId::from(subject), "test".into()).await
    };
    both_races(&pool, admin, &verb).await;
    assert_eq!(standing(&pool, subject).await.as_deref(), Some("approved"));
    verb(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin revokes");
    assert_eq!(standing(&pool, subject).await.as_deref(), Some("revoked"));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn deactivate(pool: PgPool) {
    let admin = an_admin(&pool, "sv-admin").await;
    let subject = approved(&pool, "sv-subject").await;
    let verb = move |pool: PgPool, proof: SystemAdmin| async move {
        access_service::admin_deactivate(&pool, &proof, ProfileId::from(subject)).await
    };
    both_races(&pool, admin, &verb).await;
    assert_eq!(standing(&pool, subject).await.as_deref(), Some("approved"));
    verb(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin deactivates");
    assert_eq!(
        standing(&pool, subject).await.as_deref(),
        Some("deactivated")
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reactivate(pool: PgPool) {
    let admin = an_admin(&pool, "sv-admin").await;
    let subject = approved(&pool, "sv-subject").await;
    access_service::admin_deactivate(&pool, &proof(&pool, admin).await, ProfileId::from(subject))
        .await
        .unwrap();
    let verb = move |pool: PgPool, proof: SystemAdmin| async move {
        access_service::admin_reactivate(&pool, &proof, ProfileId::from(subject)).await
    };
    both_races(&pool, admin, &verb).await;
    assert_eq!(
        standing(&pool, subject).await.as_deref(),
        Some("deactivated")
    );
    verb(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin reactivates");
    assert_eq!(standing(&pool, subject).await.as_deref(), Some("approved"));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn promote(pool: PgPool) {
    let admin = an_admin(&pool, "sv-admin").await;
    let subject = approved(&pool, "sv-subject").await;
    let t = team(&pool, "sv-team").await;
    let verb = move |pool: PgPool, proof: SystemAdmin| async move {
        access_service::promote_admin(&pool, &proof, subject, Some(t)).await
    };
    both_races(&pool, admin, &verb).await;
    assert!(
        !governs(&pool, subject).await,
        "a refused promote granted governance"
    );
    verb(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin promotes");
    assert!(governs(&pool, subject).await);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn demote(pool: PgPool) {
    let admin = an_admin(&pool, "sv-admin").await;
    let subject = an_admin(&pool, "sv-other-admin").await;
    let verb = move |pool: PgPool, proof: SystemAdmin| async move {
        access_service::demote_admin(&pool, &proof, ProfileId::from(subject)).await
    };
    both_races(&pool, admin, &verb).await;
    assert!(
        governs(&pool, subject).await,
        "a refused demote removed governance"
    );
    verb(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin demotes");
    assert!(!governs(&pool, subject).await);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reconcile_auto_join(pool: PgPool) {
    let admin = an_admin(&pool, "sv-admin").await;
    let person = approved(&pool, "sv-person").await;
    // Flagged after the approval by bare SQL, so the person is not enrolled: the drift the verb
    // repairs.
    let pool_team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name, auto_join_role) \
         VALUES ('sv-pool', 'sv-pool', 'watcher'::team_role) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let enrolled = |pool: PgPool| async move {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM kb_team_members WHERE team_id = $1 AND profile_id = $2)",
        )
        .bind(pool_team)
        .bind(person)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let verb = move |pool: PgPool, proof: SystemAdmin| async move {
        access_service::reconcile_auto_join(&pool, &proof).await
    };
    both_races(&pool, admin, &verb).await;
    assert!(
        !enrolled(pool.clone()).await,
        "a refused reconcile wrote a roster row"
    );
    verb(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin reconciles");
    assert!(enrolled(pool.clone()).await);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn close_review_request(pool: PgPool) {
    let admin = an_admin(&pool, "sv-admin").await;
    let filer = profile(&pool, "sv-filer").await;
    let review: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_principal_review_requests (profile_id, message) \
         VALUES ($1, 'please') RETURNING id",
    )
    .bind(filer)
    .fetch_one(&pool)
    .await
    .unwrap();
    let verb = move |pool: PgPool, proof: SystemAdmin| async move {
        access_service::close_review_request(
            &pool,
            &proof,
            access_service::CloseReviewRequestParams {
                request_id: review,
                decision_note: None,
            },
        )
        .await
    };
    both_races(&pool, admin, &verb).await;
    let open: bool = sqlx::query_scalar(
        "SELECT decided_at IS NULL FROM kb_principal_review_requests WHERE id = $1",
    )
    .bind(review)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(open, "a refused close decided the review");
    verb(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin closes it");
}

/// A machine credential's revocation by an admin also revokes the machine's standing (its last live
/// credential). Both halves are one act: an admin demoted mid-act is refused the whole of it, so the
/// credential is not left revoked behind a 403.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revoke_a_machine_credential(pool: PgPool) {
    let admin = an_admin(&pool, "sv-admin").await;
    let client = machine_registration_service::provision(
        &pool,
        &test_support::human_principal_for(&pool, admin).await,
        &ProvisionMachineRequest {
            client_id: "sv-machine".to_owned(),
            label: "sv-machine".to_owned(),
            owner_team_id: None,
            teams: vec![],
            grants: vec![],
        },
        AuthMode::ExternalIdp,
    )
    .await
    .expect("provision machine");
    let machine = client.profile_id;
    // Approved, so the standing arm runs: revoking its last credential revokes its standing too.
    test_support::approve(&pool, machine).await;
    let live = |pool: PgPool| async move {
        sqlx::query_scalar::<_, bool>(
            "SELECT revoked_at IS NULL FROM kb_machine_clients WHERE id = $1",
        )
        .bind(client.id)
        .fetch_one(&pool)
        .await
        .unwrap()
    };

    let (holder, pid) = demotion_in_flight(&pool, admin).await;
    let p2 = pool.clone();
    let contender = tokio::spawn(async move {
        let by = test_support::human_principal_for(&p2, admin).await;
        machine_client_service::revoke(&p2, client.id, &by).await
    });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "in flight: {err:?}");
    assert!(
        live(pool.clone()).await,
        "a refused revoke left the credential revoked"
    );
    assert_eq!(standing(&pool, machine).await.as_deref(), Some("approved"));
    test_support::grant_governance(&pool, admin).await;

    let by = test_support::human_principal_for(&pool, admin).await;
    machine_client_service::revoke(&pool, client.id, &by)
        .await
        .expect("the admin revokes it");
    assert!(!live(pool.clone()).await);
    assert_eq!(standing(&pool, machine).await.as_deref(), Some("revoked"));
}

/// A demoted admin cannot undo their own demotion with a proof minted before it: the promote is
/// refused and the governance row stays absent.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_demoted_admin_cannot_promote_themselves_back(pool: PgPool) {
    let me = an_admin(&pool, "sv-me").await;
    let t = team(&pool, "sv-team").await;
    let stale = proof(&pool, me).await;

    // Committed first.
    let (holder, _) = demotion_in_flight(&pool, me).await;
    holder.commit().await.unwrap();
    let err = access_service::promote_admin(&pool, &stale, me, Some(t))
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert!(
        !governs(&pool, me).await,
        "the demoted admin re-promoted themselves"
    );

    // In flight: the promote waits on the demotion, then is refused rather than re-inserting.
    test_support::grant_governance(&pool, me).await;
    let stale = proof(&pool, me).await;
    let (holder, pid) = demotion_in_flight(&pool, me).await;
    let p2 = pool.clone();
    let contender =
        tokio::spawn(async move { access_service::promote_admin(&p2, &stale, me, Some(t)).await });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert!(
        !governs(&pool, me).await,
        "the demoted admin re-promoted themselves"
    );
}

// ── Lock order ────────────────────────────────────────────────────────────────────────────────────

/// Two admins demote each other from one start line, many times. Each act locks both governance
/// rows in ascending id order, so neither fails on a deadlock: exactly one lands and the other is
/// refused, because its actor is no longer an admin when it reaches the lock.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn mutual_demotions_never_deadlock(pool: PgPool) {
    for round in 0..12 {
        let a = an_admin(&pool, &format!("sv-a-{round}")).await;
        let b = an_admin(&pool, &format!("sv-b-{round}")).await;
        let (pa, pb) = (proof(&pool, a).await, proof(&pool, b).await);
        let start = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let (p1, s1) = (pool.clone(), start.clone());
        let one = tokio::spawn(async move {
            s1.wait().await;
            access_service::demote_admin(&p1, &pa, ProfileId::from(b)).await
        });
        let (p2, s2) = (pool.clone(), start.clone());
        let two = tokio::spawn(async move {
            s2.wait().await;
            access_service::demote_admin(&p2, &pb, ProfileId::from(a)).await
        });
        let outcomes = [one.await.unwrap(), two.await.unwrap()];
        for o in &outcomes {
            assert!(
                matches!(o, Ok(()) | Err(ApiError::Forbidden)),
                "round {round}: {o:?}"
            );
        }
        let landed = outcomes.iter().filter(|o| o.is_ok()).count();
        assert_eq!(landed, 1, "round {round}: exactly one demotion lands");
        assert_eq!(
            governs(&pool, a).await as u8 + governs(&pool, b).await as u8,
            1,
            "round {round}: the winner keeps governance"
        );
    }
}

/// The same with Revoke, which takes each subject's standing row first and then the governance
/// pair.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn mutual_revokes_never_deadlock(pool: PgPool) {
    for round in 0..12 {
        let a = an_admin(&pool, &format!("sv-a-{round}")).await;
        let b = an_admin(&pool, &format!("sv-b-{round}")).await;
        let (pa, pb) = (proof(&pool, a).await, proof(&pool, b).await);
        let start = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let (p1, s1) = (pool.clone(), start.clone());
        let one = tokio::spawn(async move {
            s1.wait().await;
            access_service::admin_revoke(&p1, &pa, ProfileId::from(b), "race".into()).await
        });
        let (p2, s2) = (pool.clone(), start.clone());
        let two = tokio::spawn(async move {
            s2.wait().await;
            access_service::admin_revoke(&p2, &pb, ProfileId::from(a), "race".into()).await
        });
        let outcomes = [one.await.unwrap(), two.await.unwrap()];
        for o in &outcomes {
            assert!(
                matches!(o, Ok(()) | Err(ApiError::Forbidden)),
                "round {round}: {o:?}"
            );
        }
        assert_eq!(
            outcomes.iter().filter(|o| o.is_ok()).count(),
            1,
            "round {round}: exactly one revoke lands"
        );
    }
}

/// The settings write, the gating-team repoint included, asks the actor again before the exclusive
/// gating lock and any write.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn update_system_settings(pool: PgPool) {
    let admin = an_admin(&pool, "sv-admin").await;
    team(&pool, "sv-new-gate").await;
    let gate = |pool: PgPool| async move {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT gating_team_slug FROM kb_system_settings WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let before = gate(pool.clone()).await;
    let verb = move |pool: PgPool, proof: SystemAdmin| async move {
        access_service::update_system_settings(
            &pool,
            &proof,
            &temper_core::types::admin::UpdateSettingsRequest {
                gating_team_slug: Some("sv-new-gate".into()),
                instance_name: Some("renamed".into()),
                terms_version: None,
                terms_resource_uri: None,
            },
        )
        .await
    };
    both_races(&pool, admin, &verb).await;
    assert_eq!(
        gate(pool.clone()).await,
        before,
        "a refused write repointed the gating team"
    );
    verb(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin repoints it");
    assert_eq!(gate(pool.clone()).await.as_deref(), Some("sv-new-gate"));
}
