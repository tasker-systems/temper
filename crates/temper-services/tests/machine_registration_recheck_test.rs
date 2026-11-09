#![cfg(feature = "test-db")]
//! Machine registration asks the admin question again on its own transaction, before its first
//! write, with the locked governance read (`auth::is_system_admin_locked`). `rebind` takes a
//! `&SystemAdmin` proof minted on the pool, and `provision` and `issue` decide their admin bar and
//! the reach's admin arm on the pool, so before this an admin demoted after that read, or while the
//! act was in flight, still bound a client id of their choosing to a machine's reach, or registered
//! a new one.
//!
//! **In flight:** the admin's demotion runs in a transaction left open, the door is started, and
//! the test waits until the door is blocked on the demotion's backend *at the governance lock*
//! (`… kb_principal_governance … FOR KEY SHARE`), never at a pool read: the pool reads are plain
//! and never wait on an uncommitted delete. The demotion commits; the door is refused and wrote
//! nothing. Without the re-check the door never blocks and the wait times out.
//!
//! **Committed first:** the demotion commits before the door. For `rebind` the stale proof is
//! handed in, and the door is refused; without the re-check it succeeds. For `provision` and
//! `issue` the door's own pool read already sees the committed demotion, so those two tests pass
//! without the re-check too: they are regression pins on the refusal a non-admin gets, not proofs
//! of the lock.
//!
//! `rebind` also takes its source credential row `FOR UPDATE` before it checks the row, so a
//! revoke of the source cannot land between the check and the writes, and its admin re-check comes
//! before every other refusal, so a demoted admin holding a stale proof learns nothing about a
//! machine client id. Lock order: governance `KEY SHARE`, then the personal teams the machine is
//! severed from, then the credential row. A machine revoke takes the credential row after its own
//! standing and governance locks (admin) or after the owning team's row (owner); the last test races
//! the two from one start line.

use std::future::Future;

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{
    IssueMachineRequest, MachineClient, ProvisionMachineRequest, RebindMachineRequest,
};
use temper_services::auth::SystemAdmin;
use temper_services::auth_config::AuthMode;
use temper_services::error::ApiError;
use temper_services::services::{machine_client_service, machine_registration_service};
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

async fn proof(pool: &PgPool, admin: Uuid) -> SystemAdmin {
    temper_services::auth::require_system_admin_by_id(pool, ProfileId::from(admin))
        .await
        .expect("an admin's proof")
}

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("team")
}

async fn personal_team(pool: &PgPool, person: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(person)
        .fetch_one(pool)
        .await
        .expect("personal team")
}

/// An externally issued machine `admin` registered, on no team.
async fn machine(pool: &PgPool, admin: Uuid, client_id: &str) -> MachineClient {
    machine_registration_service::provision(
        pool,
        &test_support::human_principal_for(pool, admin).await,
        &ProvisionMachineRequest {
            client_id: client_id.to_owned(),
            label: client_id.to_owned(),
            owner_team_id: None,
            teams: vec![],
            grants: vec![],
        },
        AuthMode::ExternalIdp,
    )
    .await
    .expect("provision machine")
}

fn rebind_to(source: Uuid, client_id: &str, keep_old_active: bool) -> RebindMachineRequest {
    RebindMachineRequest {
        client_id: client_id.to_owned(),
        from_machine_client_id: source,
        label: client_id.to_owned(),
        keep_old_active,
    }
}

/// Whether any trace of `client_id` was written: an auth link or a registration row.
async fn registered(pool: &PgPool, client_id: &str) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_profile_auth_links WHERE auth_provider_user_id = $1) \
             OR EXISTS (SELECT 1 FROM kb_machine_clients WHERE client_id = $1)",
    )
    .bind(client_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Profiles on the instance: a refused registration creates no agent profile.
async fn agent_profiles(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_profiles")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn revoked_by(pool: &PgPool, client: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT revoked_by_profile_id FROM kb_machine_clients WHERE id = $1")
        .bind(client)
        .fetch_one(pool)
        .await
        .unwrap()
}

// ── The races ─────────────────────────────────────────────────────────────────────────────────────

async fn backend_pid(conn: &mut sqlx::PgConnection) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .expect("pid")
}

/// The statement a door blocks on at its admin re-check (`auth::is_system_admin_locked`).
const GOVERNANCE_RECHECK: &str = "%FROM kb_principal_governance%FOR KEY SHARE%";

/// The statement `rebind` blocks on when its source row is being revoked.
const SOURCE_ROW_LOCK: &str = "%FROM kb_machine_clients WHERE id = $1 FOR UPDATE%";

/// Wait until a backend is blocked on a lock `holder` holds, at a statement matching `statement`
/// (an `ILIKE` pattern). Matching the statement is what tells the transaction re-check apart from
/// any other wait: a door that waited somewhere else would not satisfy it.
async fn await_blocked_at(pool: &PgPool, holder: i32, statement: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)) AND query ILIKE $2)",
        )
        .bind(holder)
        .bind(statement)
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the door never blocked at `{statement}`: it did not take that lock"
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

/// The door, started while `admin`'s demotion is in flight, blocks at its governance re-check and
/// is refused once the demotion commits. Returns the refusal. `admin` is an admin again afterwards.
async fn refused_when_the_demotion_lands<T, F, Fut>(pool: &PgPool, admin: Uuid, door: F) -> ApiError
where
    T: std::fmt::Debug + Send + 'static,
    F: FnOnce(PgPool) -> Fut,
    Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    let (holder, pid) = demotion_in_flight(pool, admin).await;
    let contender = tokio::spawn(door(pool.clone()));
    await_blocked_at(pool, pid, GOVERNANCE_RECHECK).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    test_support::grant_governance(pool, admin).await;
    err
}

/// The door, called after `admin`'s demotion committed. Returns the refusal. `admin` is an admin
/// again afterwards.
async fn refused_after_the_demotion<T, F, Fut>(pool: &PgPool, admin: Uuid, door: F) -> ApiError
where
    T: std::fmt::Debug,
    F: FnOnce(PgPool) -> Fut,
    Fut: Future<Output = Result<T, ApiError>>,
{
    let (holder, _) = demotion_in_flight(pool, admin).await;
    holder.commit().await.unwrap();
    let err = door(pool.clone()).await.unwrap_err();
    test_support::grant_governance(pool, admin).await;
    err
}

fn is_forbidden(err: &ApiError) -> bool {
    matches!(err, ApiError::Forbidden)
}

/// `provision`'s refusal to a non-admin under an external IdP.
fn is_provision_refusal(err: &ApiError) -> bool {
    matches!(err, ApiError::ForbiddenDetail(why) if why.contains("requires a system admin"))
}

// ── rebind ────────────────────────────────────────────────────────────────────────────────────────

/// In flight: the proof was minted before the demotion; the rebind waits on it at the governance
/// lock, then is refused, and neither the new client id nor the source's revocation is written.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn rebind_in_flight(pool: PgPool) {
    let admin = an_admin(&pool, "mr-admin").await;
    let source = machine(&pool, admin, "mr-source").await;
    let stale = proof(&pool, admin).await;
    let req = rebind_to(source.id, "mr-rebound", false);

    let err = refused_when_the_demotion_lands(&pool, admin, |pool| async move {
        machine_registration_service::rebind(&pool, &stale, &req, AuthMode::ExternalIdp).await
    })
    .await;
    assert!(is_forbidden(&err), "{err:?}");
    assert!(
        !registered(&pool, "mr-rebound").await,
        "a refused rebind wrote the new client id"
    );
    assert_eq!(
        revoked_by(&pool, source.id).await,
        None,
        "a refused rebind revoked the source"
    );

    machine_registration_service::rebind(
        &pool,
        &proof(&pool, admin).await,
        &rebind_to(source.id, "mr-rebound", false),
        AuthMode::ExternalIdp,
    )
    .await
    .expect("the admin rebinds it");
}

/// Committed first: the stale proof is refused.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn rebind_committed_first(pool: PgPool) {
    let admin = an_admin(&pool, "mr-admin").await;
    let source = machine(&pool, admin, "mr-source").await;
    let stale = proof(&pool, admin).await;
    let req = rebind_to(source.id, "mr-rebound", false);

    let err = refused_after_the_demotion(&pool, admin, |pool| async move {
        machine_registration_service::rebind(&pool, &stale, &req, AuthMode::ExternalIdp).await
    })
    .await;
    assert!(is_forbidden(&err), "{err:?}");
    assert!(!registered(&pool, "mr-rebound").await);
    assert_eq!(revoked_by(&pool, source.id).await, None);
}

/// A demoted admin holding a stale proof is refused before the source row is read: a missing, a
/// revoked and a temper-issued source each answer `403`, as a non-admin is answered at the mint,
/// not the `404` or `400` an admin gets.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn rebind_refuses_a_demoted_admin_before_it_reads_the_source(pool: PgPool) {
    let admin = an_admin(&pool, "mr-admin").await;
    let revoked = machine(&pool, admin, "mr-revoked").await;
    sqlx::query("UPDATE kb_machine_clients SET revoked_at = now() WHERE id = $1")
        .bind(revoked.id)
        .execute(&pool)
        .await
        .unwrap();
    let issued = machine_registration_service::issue(
        &pool,
        &test_support::human_principal_for(&pool, admin).await,
        &IssueMachineRequest {
            label: "mr-issued".into(),
            owner_team_id: None,
            teams: vec![],
            grants: vec![],
        },
        AuthMode::ExternalIdp,
    )
    .await
    .expect("issue")
    .client;
    let stale = proof(&pool, admin).await;
    let (holder, _) = demotion_in_flight(&pool, admin).await;
    holder.commit().await.unwrap();

    for (source, what) in [
        (Uuid::now_v7(), "a missing source"),
        (revoked.id, "a revoked source"),
        (issued.id, "a temper-issued source"),
    ] {
        let err = machine_registration_service::rebind(
            &pool,
            &stale,
            &rebind_to(source, "mr-rebound", false),
            AuthMode::ExternalIdp,
        )
        .await
        .unwrap_err();
        assert!(is_forbidden(&err), "{what}: {err:?}");
    }
    // A refusal the mode gives, and the minted-namespace refusal, come after the re-check too.
    let err = machine_registration_service::rebind(
        &pool,
        &stale,
        &rebind_to(revoked.id, "tmpr_mine", false),
        AuthMode::TemperAs,
    )
    .await
    .unwrap_err();
    assert!(is_forbidden(&err), "mode and namespace: {err:?}");
}

/// A revoke of the source in flight: the rebind waits on the source row's lock, then sees the
/// revocation and is refused, and the revoked machine gets no new credential. Without the row lock
/// the rebind read the source on the pool, never waited, and bound a live client id to a machine
/// whose credential had just been revoked.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn rebind_waits_for_a_revoke_of_its_source(pool: PgPool) {
    let admin = an_admin(&pool, "mr-admin").await;
    let source = machine(&pool, admin, "mr-source").await;

    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    sqlx::query("UPDATE kb_machine_clients SET revoked_at = now() WHERE id = $1")
        .bind(source.id)
        .execute(&mut *holder)
        .await
        .unwrap();
    let by = proof(&pool, admin).await;
    let p2 = pool.clone();
    let contender = tokio::spawn(async move {
        machine_registration_service::rebind(
            &p2,
            &by,
            // The overlap window: the rebind writes nothing to the source row, so only the guard's
            // own lock can make it wait.
            &rebind_to(source.id, "mr-rebound", true),
            AuthMode::ExternalIdp,
        )
        .await
    });
    await_blocked_at(&pool, pid, SOURCE_ROW_LOCK).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(
        matches!(&err, ApiError::BadRequest(why) if why.contains("is revoked")),
        "{err:?}"
    );
    assert!(
        !registered(&pool, "mr-rebound").await,
        "a revoked machine got a new credential"
    );
}

// ── provision and issue ───────────────────────────────────────────────────────────────────────────

fn provision_req(client_id: &str) -> ProvisionMachineRequest {
    ProvisionMachineRequest {
        client_id: client_id.to_owned(),
        label: client_id.to_owned(),
        owner_team_id: None,
        teams: vec![],
        grants: vec![],
    }
}

/// In flight: provision's door and the reach's admin arm read governance on the pool and pass; the
/// transaction waits at the locked re-check, then refuses with the door's non-admin refusal, and no
/// agent profile, auth link or registration is left. No team reach is asked for, so the re-check
/// before the first write is the only governance lock the door takes.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn provision_in_flight(pool: PgPool) {
    let admin = an_admin(&pool, "mr-admin").await;
    let agents = agent_profiles(&pool).await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        machine_registration_service::provision(
            &pool,
            &by,
            &provision_req("mr-new"),
            AuthMode::ExternalIdp,
        )
        .await
    };

    let err = refused_when_the_demotion_lands(&pool, admin, door).await;
    assert!(is_provision_refusal(&err), "{err:?}");
    assert!(
        !registered(&pool, "mr-new").await,
        "a refused provision wrote the client id"
    );
    assert_eq!(
        agent_profiles(&pool).await,
        agents,
        "a refused provision created a profile"
    );

    door(pool.clone()).await.expect("the admin provisions it");
}

/// **Regression pin, passes without the re-check:** a demotion committed before `provision` is
/// seen by the door's own pool read, and the refusal is the one a non-admin gets.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn provision_committed_first(pool: PgPool) {
    let admin = an_admin(&pool, "mr-admin").await;
    let err = refused_after_the_demotion(&pool, admin, move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        machine_registration_service::provision(
            &pool,
            &by,
            &provision_req("mr-new"),
            AuthMode::ExternalIdp,
        )
        .await
    })
    .await;
    assert!(is_provision_refusal(&err), "{err:?}");
    assert!(!registered(&pool, "mr-new").await);
}

fn issue_req(owner_team_id: Option<Uuid>) -> IssueMachineRequest {
    IssueMachineRequest {
        label: "mr-issued".into(),
        owner_team_id,
        teams: vec![],
        grants: vec![],
    }
}

/// In flight: a teamless credential is admin-only to issue. The transaction waits at the locked
/// re-check, then refuses (`403`, what a caller with no authority gets), and nothing is minted.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn issue_in_flight(pool: PgPool) {
    let admin = an_admin(&pool, "mr-admin").await;
    let agents = agent_profiles(&pool).await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        machine_registration_service::issue(&pool, &by, &issue_req(None), AuthMode::TemperAs).await
    };

    let err = refused_when_the_demotion_lands(&pool, admin, door).await;
    assert!(is_forbidden(&err), "{err:?}");
    assert_eq!(
        agent_profiles(&pool).await,
        agents,
        "a refused issue created a profile"
    );

    door(pool.clone()).await.expect("the admin issues it");
}

/// **Regression pin, passes without the re-check:** a demotion committed before `issue` is seen by
/// the authority's own pool read.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn issue_committed_first(pool: PgPool) {
    let admin = an_admin(&pool, "mr-admin").await;
    let agents = agent_profiles(&pool).await;
    let err = refused_after_the_demotion(&pool, admin, move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        machine_registration_service::issue(&pool, &by, &issue_req(None), AuthMode::TemperAs).await
    })
    .await;
    assert!(is_forbidden(&err), "{err:?}");
    assert_eq!(agent_profiles(&pool).await, agents);
}

/// A demoted admin who also owns the machine's team is refused the in-flight act, which was
/// authorized on the admin arm; a retry decides on the owner arm and is admitted.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn issue_by_a_demoted_admin_who_owns_the_team(pool: PgPool) {
    let admin = an_admin(&pool, "mr-admin").await;
    let t = team(&pool, "mr-team").await;
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
        machine_registration_service::issue(&p2, &by, &issue_req(Some(t)), AuthMode::TemperAs).await
    });
    await_blocked_at(&pool, pid, GOVERNANCE_RECHECK).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(is_forbidden(&err), "{err:?}");

    let by = test_support::human_principal_for(&pool, admin).await;
    machine_registration_service::issue(&pool, &by, &issue_req(Some(t)), AuthMode::TemperAs)
        .await
        .expect("the owner issues it");
}

// ── Lock order ────────────────────────────────────────────────────────────────────────────────────

/// A rebind and an admin's revoke of the same source, from one start line, many times: a lock-order
/// witness, not a proof. The machine sits on a person's personal team, so the rebind takes
/// governance, then that team and its member row (the severance), then the credential row; the
/// revoke takes the machine's standing row, the revoking admin's governance row, then the
/// credential row, then the standing act. No round in the run may deadlock (a deadlock aborts one
/// side, which fails the round), and each round's state must match the order that landed. The
/// revoke runs pool reads before its transaction, so the rebind-first order is the usual one; the
/// revoke-first order is pinned by construction in `rebind_waits_for_a_revoke_of_its_source`. The
/// two orders:
///
/// - the rebind first: it revoked the source itself, its new credential is live, and the revoke,
///   finding the source revoked and another credential live, leaves standing alone;
/// - the revoke first: the rebind sees the revoked source under its lock and is refused, and the
///   machine gets no new credential.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn rebind_and_revoke_of_one_source_serialize(pool: PgPool) {
    let rebinder = an_admin(&pool, "mr-rebinder").await;
    let revoker = an_admin(&pool, "mr-revoker").await;
    let person = profile(&pool, "mr-person").await;
    test_support::approve(&pool, person).await;
    let theirs = personal_team(&pool, person).await;

    for round in 0..12 {
        let source = machine(&pool, rebinder, &format!("mr-source-{round}")).await;
        test_support::approve(&pool, source.profile_id).await;
        // The person's own machine (a personal team admits only those), registered for them.
        sqlx::query("UPDATE kb_machine_clients SET registered_by_profile_id = $2 WHERE id = $1")
            .bind(source.id)
            .bind(person)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(theirs)
        .bind(source.profile_id)
        .execute(&pool)
        .await
        .unwrap();
        let new_id = format!("mr-rebound-{round}");

        let start = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let (p1, s1, req) = (
            pool.clone(),
            start.clone(),
            rebind_to(source.id, &new_id, false),
        );
        let by = proof(&pool, rebinder).await;
        let rebind = tokio::spawn(async move {
            s1.wait().await;
            machine_registration_service::rebind(&p1, &by, &req, AuthMode::ExternalIdp).await
        });
        let (p2, s2) = (pool.clone(), start.clone());
        let revoke = tokio::spawn(async move {
            let by = test_support::human_principal_for(&p2, revoker).await;
            s2.wait().await;
            machine_client_service::revoke(&p2, source.id, &by).await
        });
        let (rebound, revoked) = (rebind.await.unwrap(), revoke.await.unwrap());
        revoked.unwrap_or_else(|e| panic!("round {round}: the revoke failed: {e:?}"));
        match rebound {
            Ok(client) => {
                assert_eq!(
                    revoked_by(&pool, source.id).await,
                    Some(rebinder),
                    "round {round}: rebound after the revoke committed"
                );
                assert_eq!(revoked_by(&pool, client.id).await, None, "round {round}");
            }
            Err(ApiError::BadRequest(why)) if why.contains("is revoked") => {
                assert_eq!(
                    revoked_by(&pool, source.id).await,
                    Some(revoker),
                    "round {round}"
                );
                assert!(!registered(&pool, &new_id).await, "round {round}");
            }
            Err(other) => panic!("round {round}: not one serial order: {other:?}"),
        }
    }
}
