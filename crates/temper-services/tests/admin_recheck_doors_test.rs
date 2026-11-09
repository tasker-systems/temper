#![cfg(feature = "test-db")]
//! The team doors whose admin arm was decided on the pool alone now ask it again on their write
//! transaction, under their own locks, with the locked governance read
//! (`auth::require_system_admin_locked`). A demotion deletes the admin's `kb_principal_governance`
//! row; the re-check takes that row `FOR KEY SHARE`, so a demotion either committed first (the door
//! is refused) or waits until the door's transaction ends.
//!
//! Each door is raced twice:
//!
//! - **in flight:** the demotion runs in a transaction left open, the door is started, the test
//!   waits until the door is blocked on the demotion's backend, and then the demotion commits. The
//!   door is refused and writes nothing. Before the re-check the door never blocked: its pool check
//!   had seen the not-yet-deleted row, and it wrote an admin act beside a demotion already decided.
//! - **committed first:** the demotion commits before the door is called; the door is refused.
//!
//! Lock order: each door takes its own team, member, request or invitation rows first and the
//! governance row after them. A demotion takes none of those rows, before or after its delete, so
//! no order of the two can wait in a cycle. The last test races a full revoke of the reviewing admin
//! (which locks the admin's standing row before its delete) against a join-request decision (which
//! locks the requester's standing row, then the gating team, then the governance row), the pairing
//! with the most locks on both sides.

use std::future::Future;

use sqlx::PgPool;
use temper_core::types::access_gate::JoinRequestStatus;
use temper_core::types::ids::ProfileId;
use temper_core::types::team::{ReparentRequest, TeamCreateRequest, TeamRole, TeamUpdateRequest};
use temper_principal::Provisioner;
use temper_services::error::ApiError;
use temper_services::services::{
    access_service, invitation_service, standing_service, team_service,
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

/// An approved system admin on no team.
async fn admin(pool: &PgPool) -> Uuid {
    let id = profile(pool, "ra-admin").await;
    test_support::approved_admin(pool, id).await;
    id
}

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("team")
}

async fn link(pool: &PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .expect("link");
}

async fn linked(pool: &PgPool, parent: Uuid, child: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_teams_parents WHERE parent_id = $1 AND child_id = $2)",
    )
    .bind(parent)
    .bind(child)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn live(pool: &PgPool, team: Uuid) -> bool {
    sqlx::query_scalar("SELECT is_active FROM kb_teams WHERE id = $1")
        .bind(team)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn root(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(pool)
        .await
        .expect("the root")
}

async fn team_name(pool: &PgPool, team: Uuid) -> String {
    sqlx::query_scalar("SELECT name FROM kb_teams WHERE id = $1")
        .bind(team)
        .fetch_one(pool)
        .await
        .unwrap()
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
            "the door never blocked on the demotion: it did not re-check under its locks"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// `who`'s demotion, in a transaction left open: the governance row is deleted and locked until the
/// holder commits. `principal_governance_set` is what `access_service::demote_admin` and
/// `standing_service::apply` (on Revoke and Deactivate) both run.
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

/// The door, started while `admin`'s demotion is in flight, waits on it and is refused once it
/// commits. `admin` is an admin again afterwards.
async fn refused_when_the_demotion_lands<T, F, Fut>(pool: &PgPool, admin: Uuid, door: F)
where
    T: std::fmt::Debug + Send + 'static,
    F: FnOnce(PgPool) -> Fut,
    Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    let (holder, pid) = demotion_in_flight(pool, admin).await;
    let contender = tokio::spawn(door(pool.clone()));
    await_blocked_by(pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "in flight: {err:?}");
    test_support::grant_governance(pool, admin).await;
}

/// The door, called after `admin`'s demotion committed, is refused. `admin` is an admin again
/// afterwards.
async fn refused_after_the_demotion<T, F, Fut>(pool: &PgPool, admin: Uuid, door: F)
where
    T: std::fmt::Debug,
    F: FnOnce(PgPool) -> Fut,
    Fut: Future<Output = Result<T, ApiError>>,
{
    let (holder, _) = demotion_in_flight(pool, admin).await;
    holder.commit().await.unwrap();
    let err = door(pool.clone()).await.unwrap_err();
    assert!(
        matches!(err, ApiError::Forbidden),
        "committed first: {err:?}"
    );
    test_support::grant_governance(pool, admin).await;
}

/// Both races. Each test then runs the door once more as its control: the restored admin passes.
async fn both_races<T, F, Fut>(pool: &PgPool, admin: Uuid, door: F)
where
    T: std::fmt::Debug + Send + 'static,
    F: Fn(PgPool) -> Fut,
    Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    refused_when_the_demotion_lands(pool, admin, &door).await;
    refused_after_the_demotion(pool, admin, &door).await;
}

// ── The doors ─────────────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn create_with_auto_join(pool: PgPool) {
    let admin = admin(&pool).await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        team_service::create_team(
            &pool,
            &by,
            &TeamCreateRequest {
                slug: "ra-pool".into(),
                name: None,
                parent: None,
                parents: None,
                auto_join_role: Some(TeamRole::Watcher),
            },
        )
        .await
    };
    both_races(&pool, admin, door).await;
    let made: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM kb_teams WHERE slug = 'ra-pool')")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!made, "a refused create wrote the team");
    door(pool.clone()).await.expect("the admin creates it");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn update_of_an_instance_team(pool: PgPool) {
    let admin = admin(&pool).await;
    let root = root(&pool).await;
    let before = team_name(&pool, root).await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        team_service::update_team(
            &pool,
            &by,
            root,
            &TeamUpdateRequest {
                name: Some("renamed".into()),
                description: None,
            },
        )
        .await
    };
    both_races(&pool, admin, door).await;
    assert_eq!(team_name(&pool, root).await, before);
    door(pool.clone()).await.expect("the admin updates it");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn delete(pool: PgPool) {
    let admin = admin(&pool).await;
    let t = team(&pool, "ra-doomed").await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        team_service::delete_team(&pool, &by, t).await
    };
    both_races(&pool, admin, door).await;
    assert!(live(&pool, t).await, "a refused delete killed the team");
    door(pool.clone()).await.expect("the admin deletes it");
}

/// A demoted admin who also owns the team deletes it as its owner: the re-check falls back to the
/// owner arm under the same lock.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn delete_by_a_demoted_admin_who_owns_the_team(pool: PgPool) {
    let admin = admin(&pool).await;
    let t = team(&pool, "ra-owned").await;
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')")
        .bind(t)
        .bind(admin)
        .execute(&pool)
        .await
        .unwrap();
    let (holder, pid) = demotion_in_flight(&pool, admin).await;
    let p2 = pool.clone();
    let contender = tokio::spawn(async move {
        let by = test_support::human_principal_for(&p2, admin).await;
        team_service::delete_team(&p2, &by, t).await
    });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();
    contender.await.unwrap().expect("the owner deletes it");
    assert!(!live(&pool, t).await);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn restore(pool: PgPool) {
    let admin = admin(&pool).await;
    let t = team(&pool, "ra-dead").await;
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(t)
        .execute(&pool)
        .await
        .unwrap();
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        team_service::restore_team(&pool, &by, t).await
    };
    both_races(&pool, admin, door).await;
    assert!(!live(&pool, t).await, "a refused restore revived the team");
    door(pool.clone()).await.expect("the admin restores it");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn detach(pool: PgPool) {
    let admin = admin(&pool).await;
    let (p, c) = (
        team(&pool, "ra-parent").await,
        team(&pool, "ra-child").await,
    );
    link(&pool, p, c).await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        team_service::detach_child(&pool, &by, p, c).await
    };
    both_races(&pool, admin, door).await;
    assert!(linked(&pool, p, c).await, "a refused detach cut the link");
    door(pool.clone()).await.expect("the admin detaches it");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn attach(pool: PgPool) {
    let admin = admin(&pool).await;
    let (p, c) = (
        team(&pool, "ra-parent").await,
        team(&pool, "ra-child").await,
    );
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        team_service::attach_child(&pool, &by, p, c).await
    };
    both_races(&pool, admin, door).await;
    assert!(
        !linked(&pool, p, c).await,
        "a refused attach wrote the link"
    );
    door(pool.clone()).await.expect("the admin attaches it");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reparent(pool: PgPool) {
    let admin = admin(&pool).await;
    let from = team(&pool, "ra-from").await;
    let to = team(&pool, "ra-to").await;
    let c = team(&pool, "ra-child").await;
    link(&pool, from, c).await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        team_service::reparent(
            &pool,
            &by,
            c,
            &ReparentRequest {
                from: "+ra-from".into(),
                to: "+ra-to".into(),
            },
        )
        .await
    };
    both_races(&pool, admin, door).await;
    assert!(linked(&pool, from, c).await && !linked(&pool, to, c).await);
    door(pool.clone()).await.expect("the admin moves it");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn set_auto_join(pool: PgPool) {
    let admin = admin(&pool).await;
    let t = team(&pool, "ra-pool").await;
    // The proof is minted on the pool before the race, as the handler mints it.
    let door = move |pool: PgPool| async move {
        let proof = test_support::system_admin_proof_for(&pool, admin).await;
        team_service::set_auto_join(&pool, &proof, t, Some(TeamRole::Watcher)).await
    };
    // `system_admin_proof_for` itself refuses once the demotion committed, so the committed-first
    // race mints before the demotion.
    refused_when_the_demotion_lands(&pool, admin, door).await;
    let proof = test_support::system_admin_proof_for(&pool, admin).await;
    let (holder, _) = demotion_in_flight(&pool, admin).await;
    holder.commit().await.unwrap();
    let err = team_service::set_auto_join(&pool, &proof, t, Some(TeamRole::Watcher))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ApiError::Forbidden),
        "committed first: {err:?}"
    );
    test_support::grant_governance(&pool, admin).await;
    let role: Option<String> =
        sqlx::query_scalar("SELECT auto_join_role::text FROM kb_teams WHERE id = $1")
            .bind(t)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(role, None, "a refused set wrote the role");
    door(pool.clone()).await.expect("the admin sets it");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revoke_invitation(pool: PgPool) {
    let admin = admin(&pool).await;
    let t = team(&pool, "ra-team").await;
    let inv: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_team_invitations (id, team_id, invited_email, invited_by_profile_id, role, token) \
         VALUES (uuid_generate_v7(), $1, 'x@example.com', $2, 'member', 'ra-token') RETURNING id",
    )
    .bind(t)
    .bind(admin)
    .fetch_one(&pool)
    .await
    .unwrap();
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        invitation_service::revoke_invitation(&pool, &by, t, inv).await
    };
    both_races(&pool, admin, door).await;
    let revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM kb_team_invitations WHERE id = $1")
            .bind(inv)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!revoked, "a refused revoke revoked the invitation");
    door(pool.clone()).await.expect("the admin revokes it");
}

// ── The join-request decision ────────────────────────────────────────────────────────────────────

async fn gating_team(pool: &PgPool) {
    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ('ra-gate','Gate')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'ra-gate' WHERE id = 1")
        .execute(pool)
        .await
        .unwrap();
}

/// A person born `Denied` with a pending join request filed through the real door.
async fn requester(pool: &PgPool, handle: &str) -> Uuid {
    let id = profile(pool, handle).await;
    standing_service::provision(pool, ProfileId::from(id), Provisioner::OauthFirstLogin)
        .await
        .expect("born denied");
    access_service::create_join_request(
        pool,
        &test_support::human_principal_for(pool, id).await,
        access_service::CreateJoinRequestParams {
            message: None,
            source: "test".to_string(),
            accepted_terms_version: None,
        },
        None,
    )
    .await
    .expect("file a join request")
    .id
}

async fn request_status(pool: &PgPool, request: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM kb_join_requests WHERE id = $1")
        .bind(request)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn decide_a_join_request(pool: PgPool) {
    let admin = admin(&pool).await;
    gating_team(&pool).await;
    let request = requester(&pool, "ra-requester").await;
    let decide = move |pool: PgPool| async move {
        let proof = test_support::system_admin_proof_for(&pool, admin).await;
        access_service::review_request(
            &pool,
            &proof,
            access_service::ReviewRequestParams {
                request_id: request,
                decision: JoinRequestStatus::Approved,
                decision_note: None,
            },
        )
        .await
    };
    refused_when_the_demotion_lands(&pool, admin, decide).await;
    let proof = test_support::system_admin_proof_for(&pool, admin).await;
    let (holder, _) = demotion_in_flight(&pool, admin).await;
    holder.commit().await.unwrap();
    let err = access_service::review_request(
        &pool,
        &proof,
        access_service::ReviewRequestParams {
            request_id: request,
            decision: JoinRequestStatus::Approved,
            decision_note: None,
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, ApiError::Forbidden),
        "committed first: {err:?}"
    );
    test_support::grant_governance(&pool, admin).await;
    assert_eq!(request_status(&pool, request).await, "pending");
    decide(pool.clone()).await.expect("the admin approves it");
    assert_eq!(request_status(&pool, request).await, "approved");
}

/// The pairing with the most locks on both sides: another admin revokes the reviewing admin (the
/// revoke locks the reviewer's standing row, then deletes their governance row) while the reviewer
/// decides a join request (the requester's standing row, the request, the gating team and its
/// member row, then the reviewer's governance row). Run many times from one start line: neither side
/// ever fails on a deadlock, and the outcome is one of the two serial orders.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_revoke_of_the_reviewer_and_the_decision_never_deadlock(pool: PgPool) {
    gating_team(&pool).await;
    let other = profile(&pool, "ra-other-admin").await;
    test_support::approved_admin(&pool, other).await;
    for round in 0..12 {
        let reviewer = profile(&pool, &format!("ra-reviewer-{round}")).await;
        test_support::approved_admin(&pool, reviewer).await;
        let request = requester(&pool, &format!("ra-req-{round}")).await;
        let reviewer_proof = test_support::system_admin_proof_for(&pool, reviewer).await;
        let other_proof = test_support::system_admin_proof_for(&pool, other).await;

        let start = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let (p1, s1) = (pool.clone(), start.clone());
        let decide = tokio::spawn(async move {
            s1.wait().await;
            access_service::review_request(
                &p1,
                &reviewer_proof,
                access_service::ReviewRequestParams {
                    request_id: request,
                    decision: JoinRequestStatus::Approved,
                    decision_note: None,
                },
            )
            .await
        });
        let (p2, s2) = (pool.clone(), start.clone());
        let revoke = tokio::spawn(async move {
            s2.wait().await;
            access_service::admin_revoke(
                &p2,
                &other_proof,
                ProfileId::from(reviewer),
                "race".into(),
            )
            .await
        });
        let decided = decide.await.unwrap();
        revoke.await.unwrap().expect("the revoke never fails");
        match decided {
            Ok(_) => assert_eq!(request_status(&pool, request).await, "approved"),
            Err(ApiError::Forbidden) => {
                assert_eq!(request_status(&pool, request).await, "pending");
            }
            Err(other) => panic!("round {round}: the decision failed otherwise: {other:?}"),
        }
    }
}

// ── The fallbacks and the manager re-checks ───────────────────────────────────────────────────────

async fn join(pool: &PgPool, team: Uuid, who: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role) \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team)
    .bind(who)
    .bind(role)
    .execute(pool)
    .await
    .expect("seed member");
}

/// A membership change in a transaction left open, under the team's row lock (the lock every
/// membership writer takes): `who`'s row on `team` is set to `role`, or removed when `None`.
async fn membership_change_in_flight(
    pool: &PgPool,
    team: Uuid,
    who: Uuid,
    role: Option<&str>,
) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(team)
        .execute(&mut *holder)
        .await
        .unwrap();
    match role {
        Some(role) => {
            sqlx::query(
                "UPDATE kb_team_members SET role = $3::team_role \
                 WHERE team_id = $1 AND profile_id = $2",
            )
            .bind(team)
            .bind(who)
            .bind(role)
            .execute(&mut *holder)
            .await
            .unwrap();
        }
        None => {
            sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
                .bind(team)
                .bind(who)
                .execute(&mut *holder)
                .await
                .unwrap();
        }
    }
    (holder, pid)
}

/// An admin demoted mid-detach who also manages the child is re-decided on the child arm under the
/// same locks, and detaches.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn detach_by_a_demoted_admin_who_manages_the_child(pool: PgPool) {
    let admin = admin(&pool).await;
    let (p, c) = (
        team(&pool, "ra-parent").await,
        team(&pool, "ra-child").await,
    );
    link(&pool, p, c).await;
    join(&pool, c, admin, "maintainer").await;
    let (holder, pid) = demotion_in_flight(&pool, admin).await;
    let p2 = pool.clone();
    let contender = tokio::spawn(async move {
        let by = test_support::human_principal_for(&p2, admin).await;
        team_service::detach_child(&p2, &by, p, c).await
    });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();
    contender
        .await
        .unwrap()
        .expect("the child's maintainer detaches it");
    assert!(!linked(&pool, p, c).await);
}

/// The manager arm of `update_team` on an ordinary team, decided again under the team lock: a
/// manager demoted to member after the pool check is refused, and the name is unchanged.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn update_by_a_manager_demoted_after_the_pool_check(pool: PgPool) {
    let manager = profile(&pool, "ra-manager").await;
    test_support::approve(&pool, manager).await;
    let t = team(&pool, "ra-team").await;
    join(&pool, t, manager, "maintainer").await;
    let (holder, pid) = membership_change_in_flight(&pool, t, manager, Some("member")).await;
    let p2 = pool.clone();
    let contender = tokio::spawn(async move {
        let by = test_support::human_principal_for(&p2, manager).await;
        team_service::update_team(
            &p2,
            &by,
            t,
            &TeamUpdateRequest {
                name: Some("renamed".into()),
                description: None,
            },
        )
        .await
    });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert_eq!(team_name(&pool, t).await, "ra-team");
}

async fn invitation(pool: &PgPool, team: Uuid, by: Uuid, email: &str, token: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_team_invitations (id, team_id, invited_email, invited_by_profile_id, role, token) \
         VALUES (uuid_generate_v7(), $1, $2, $3, 'member', $4) RETURNING id",
    )
    .bind(team)
    .bind(email)
    .bind(by)
    .bind(token)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn invitation_revoked(pool: &PgPool, inv: Uuid) -> bool {
    sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM kb_team_invitations WHERE id = $1")
        .bind(inv)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// `revoke_invitation` re-runs the whole membership authority under the team lock: a manager
/// removed from the team after the pool check is refused, and the invitation stays live.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revoke_invitation_by_a_manager_removed_after_the_pool_check(pool: PgPool) {
    let manager = profile(&pool, "ra-manager").await;
    test_support::approve(&pool, manager).await;
    let t = team(&pool, "ra-team").await;
    join(&pool, t, manager, "owner").await;
    let inv = invitation(&pool, t, manager, "x@example.com", "ra-token").await;
    let (holder, pid) = membership_change_in_flight(&pool, t, manager, None).await;
    let p2 = pool.clone();
    let contender = tokio::spawn(async move {
        let by = test_support::human_principal_for(&p2, manager).await;
        invitation_service::revoke_invitation(&p2, &by, t, inv).await
    });
    await_blocked_by(&pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert!(!invitation_revoked(&pool, inv).await);
}

/// Revoke and accept both take the team row before the invitation row, so they serialize: from one
/// start line, exactly one of them wins, and the roster agrees with which.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revoke_and_accept_of_one_invitation_serialize(pool: PgPool) {
    let owner = profile(&pool, "ra-owner").await;
    test_support::approve(&pool, owner).await;
    for round in 0..12 {
        let t = team(&pool, &format!("ra-team-{round}")).await;
        join(&pool, t, owner, "owner").await;
        let invitee = profile(&pool, &format!("ra-invitee-{round}")).await;
        test_support::approve(&pool, invitee).await;
        let email = format!("invitee-{round}@example.com");
        sqlx::query(
            "INSERT INTO kb_profile_auth_links \
               (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default) \
             VALUES (gen_random_uuid(), $1, 'test', $2, $3, true, true)",
        )
        .bind(invitee)
        .bind(format!("uid-{round}"))
        .bind(&email)
        .execute(&pool)
        .await
        .unwrap();
        let token = format!("ra-token-{round}");
        let inv = invitation(&pool, t, owner, &email, &token).await;

        let start = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let (p1, s1) = (pool.clone(), start.clone());
        let revoke = tokio::spawn(async move {
            let by = test_support::human_principal_for(&p1, owner).await;
            s1.wait().await;
            invitation_service::revoke_invitation(&p1, &by, t, inv).await
        });
        let (p2, s2, tok) = (pool.clone(), start.clone(), token.clone());
        let accept = tokio::spawn(async move {
            let by = test_support::human_principal_for(&p2, invitee).await;
            s2.wait().await;
            invitation_service::accept_invitation(&p2, &by, &tok).await
        });
        let (revoked, accepted) = (revoke.await.unwrap(), accept.await.unwrap());
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM kb_team_members WHERE team_id = $1 AND profile_id = $2)",
        )
        .bind(t)
        .bind(invitee)
        .fetch_one(&pool)
        .await
        .unwrap();
        match (&revoked, &accepted) {
            (Ok(()), Err(ApiError::BadRequest(_) | ApiError::NotFound(_))) => {
                assert!(!member, "round {round}: revoked, yet the invitee joined");
                assert!(invitation_revoked(&pool, inv).await);
            }
            (Err(ApiError::BadRequest(_)), Ok(_)) => {
                assert!(
                    member,
                    "round {round}: accepted, yet the invitee is not on the team"
                );
                assert!(!invitation_revoked(&pool, inv).await);
            }
            other => panic!("round {round}: not one serial order: {other:?}"),
        }
    }
}
