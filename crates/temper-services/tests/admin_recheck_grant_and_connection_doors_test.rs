#![cfg(feature = "test-db")]
//! The grant door, the machine-client doors beside revoke, and the connection doors decided their
//! system-admin arm on a pool read (`Principal::system_admin`) and never asked again on their write.
//! Each now asks again first on its transaction, before any write, with the locked governance read
//! (`auth::admin_arm_holds`, over `auth::is_system_admin_locked`), and a demoted admin is refused as
//! a caller with no authority at that door is: `403` at the grant door and at connection
//! provisioning, the missing-id `404` sentence at every per-row door.
//!
//! **In flight:** the admin's demotion runs in a transaction left open, the door is started, and the
//! test waits until it is blocked on the demotion's backend at the governance lock
//! (`… kb_principal_governance … FOR KEY SHARE`). The pool reads ahead of it are plain and never wait
//! on an uncommitted delete, so the match can only be the transaction re-check. The demotion
//! commits; the door is refused and wrote nothing. Without the re-check the door never blocks and
//! the wait times out: **every in-flight test fails on the code before this change.**
//!
//! **Committed first:** these doors take a `HumanPrincipal`, not a proof, so their own pool read
//! already sees a committed demotion. Those halves are **regression pins on the refusal text**:
//! they pass on the code before this change. Machine revoke's test has only an in-flight half.
//!
//! Lock order: governance first, then the door's own rows (the subject floor's resource row, a
//! grant row, the severed personal teams, the credential or connection row). A demotion locks the
//! subject's standing and governance rows and none of those. The last test races a secret rotation
//! by one admin against another admin's revoke of the first, so the two contend on the rotating
//! admin's governance row.

use std::future::Future;

use sqlx::PgPool;
use temper_core::types::cognitive_maps::{GrantCapabilityRequest, RevokeCapabilityRequest};
use temper_core::types::connection::{
    Connection, ConnectionCredential, ProvisionConnectionRequest,
};
use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{IssueMachineRequest, MachineClient};
use temper_services::auth_config::AuthMode;
use temper_services::broker::FakeBroker;
use temper_services::error::ApiError;
use temper_services::services::{
    access_service, connection_service, machine_client_service, machine_registration_service,
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

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("team")
}

async fn team_context(pool: &PgPool, team: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(team)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("context")
}

async fn grant_rows(pool: &PgPool, subject: Uuid, team: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants WHERE subject_id = $1 \
           AND principal_table = 'kb_teams' AND principal_id = $2",
    )
    .bind(subject)
    .bind(team)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A temper-issued, teamless machine credential: admin-only to act on.
async fn issued_machine(pool: &PgPool, admin: Uuid) -> MachineClient {
    machine_registration_service::issue(
        pool,
        &test_support::human_principal_for(pool, admin).await,
        &IssueMachineRequest {
            label: "ra-machine".into(),
            owner_team_id: None,
            teams: vec![],
            grants: vec![],
        },
        AuthMode::TemperAs,
    )
    .await
    .expect("issue")
    .client
}

async fn secret_hash(pool: &PgPool, client: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT secret_hash FROM kb_machine_clients WHERE id = $1")
        .bind(client)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn connection_req(name: &str) -> ProvisionConnectionRequest {
    ProvisionConnectionRequest {
        provider: "github".into(),
        name: name.into(),
        owner_team_id: None,
        reach_granularity: None,
        reach_covers: None,
    }
}

/// A teamless connection: admin-only to act on.
async fn connection(pool: &PgPool, admin: Uuid, name: &str) -> Connection {
    connection_service::provision(
        pool,
        &test_support::human_principal_for(pool, admin).await,
        &connection_req(name),
    )
    .await
    .expect("provision connection")
}

async fn connection_row(pool: &PgPool, id: Uuid) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('revoked', revoked_at IS NOT NULL, 'credential', credential, \
                'webhook_events', webhook_events, 'tool_manifest', tool_manifest) \
           FROM kb_connections WHERE id = $1",
    )
    .bind(id)
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

/// The statement a door blocks on at its admin re-check (`auth::is_system_admin_locked`).
const GOVERNANCE_RECHECK: &str = "%FROM kb_principal_governance%FOR KEY SHARE%";

/// Wait until a backend is blocked on a lock `holder` holds, at a statement matching `statement`.
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
            "the door never blocked at `{statement}`: it did not re-check under the lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// `who`'s demotion, in a transaction left open.
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

/// Both races, each asserting `refused` on the answer. `admin` is an admin again afterwards.
async fn both_races<T, F, Fut>(pool: &PgPool, admin: Uuid, refused: fn(&ApiError) -> bool, door: F)
where
    T: std::fmt::Debug + Send + 'static,
    F: Fn(PgPool) -> Fut,
    Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    let (holder, pid) = demotion_in_flight(pool, admin).await;
    let contender = tokio::spawn(door(pool.clone()));
    await_blocked_at(pool, pid, GOVERNANCE_RECHECK).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(refused(&err), "in flight: {err:?}");
    test_support::grant_governance(pool, admin).await;

    let (holder, _) = demotion_in_flight(pool, admin).await;
    holder.commit().await.unwrap();
    let err = door(pool.clone()).await.unwrap_err();
    assert!(refused(&err), "committed first: {err:?}");
    test_support::grant_governance(pool, admin).await;
}

fn forbidden(err: &ApiError) -> bool {
    matches!(err, ApiError::Forbidden)
}

fn machine_client_refusal(err: &ApiError) -> bool {
    matches!(err, ApiError::NotFound(why) if why == "machine client not found or not readable")
}

fn connection_refusal(err: &ApiError) -> bool {
    matches!(err, ApiError::NotFound(why) if why == "connection not found or not readable")
}

// ── Machine clients ───────────────────────────────────────────────────────────────────────────────

/// A rotation hands back a live secret for the machine. A demoted admin gets none, and the stored
/// secret is unchanged.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn rotate_secret(pool: PgPool) {
    let admin = an_admin(&pool, "ra-admin").await;
    let client = issued_machine(&pool, admin).await;
    let before = secret_hash(&pool, client.id).await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        machine_client_service::rotate_secret(&pool, &by, client.id, 0, AuthMode::TemperAs).await
    };
    both_races(&pool, admin, machine_client_refusal, door).await;
    assert_eq!(
        secret_hash(&pool, client.id).await,
        before,
        "a refused rotation rotated"
    );
    door(pool.clone()).await.expect("the admin rotates it");
    assert_ne!(secret_hash(&pool, client.id).await, before);
}

/// Machine revoke already re-checked its admin arm; a demoted admin is now refused with the
/// missing-id sentence a caller with no authority gets, where it was `403` (an existence signal).
/// In flight only, and it fails on the code before this change (`403`). Committed first, the
/// pool's `authorize_control` already answered with the missing-id sentence.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revoke_machine_credential_refuses_as_a_missing_id(pool: PgPool) {
    let admin = an_admin(&pool, "ra-admin").await;
    let client = issued_machine(&pool, admin).await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        machine_client_service::revoke(&pool, client.id, &by).await
    };
    // In flight only: committed first, the pool's `authorize_control` refuses before the re-check.
    let (holder, pid) = demotion_in_flight(&pool, admin).await;
    let contender = tokio::spawn(door(pool.clone()));
    await_blocked_at(&pool, pid, GOVERNANCE_RECHECK).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(machine_client_refusal(&err), "{err:?}");
}

// ── The grant door ────────────────────────────────────────────────────────────────────────────────

/// A grant outlives the demotion of the admin who made it: a demoted admin writes none.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn grant_capability(pool: PgPool) {
    let admin = an_admin(&pool, "ra-admin").await;
    let owner = team(&pool, "ra-owner").await;
    let to = team(&pool, "ra-recipient").await;
    let ctx = team_context(&pool, owner, "ra-ctx").await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        access_service::grant_capability(
            &pool,
            &by,
            &GrantCapabilityRequest {
                subject_table: "kb_contexts".into(),
                subject_id: ctx,
                principal_table: "kb_teams".into(),
                principal_id: to,
                can_read: true,
                can_write: true,
                can_delete: false,
                can_grant: false,
            },
        )
        .await
    };
    both_races(&pool, admin, forbidden, door).await;
    assert_eq!(
        grant_rows(&pool, ctx, to).await,
        0,
        "a refused grant wrote a row"
    );
    door(pool.clone()).await.expect("the admin grants it");
    assert_eq!(grant_rows(&pool, ctx, to).await, 1);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revoke_capability(pool: PgPool) {
    let admin = an_admin(&pool, "ra-admin").await;
    let owner = team(&pool, "ra-owner").await;
    let to = team(&pool, "ra-recipient").await;
    let ctx = team_context(&pool, owner, "ra-ctx").await;
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_teams', $2, true, $3)",
    )
    .bind(ctx)
    .bind(to)
    .bind(admin)
    .execute(&pool)
    .await
    .unwrap();
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        access_service::revoke_capability(
            &pool,
            &by,
            &RevokeCapabilityRequest {
                subject_table: "kb_contexts".into(),
                subject_id: ctx,
                principal_table: "kb_teams".into(),
                principal_id: to,
            },
        )
        .await
    };
    both_races(&pool, admin, forbidden, door).await;
    assert_eq!(
        grant_rows(&pool, ctx, to).await,
        1,
        "a refused revoke deleted the grant"
    );
    door(pool.clone()).await.expect("the admin revokes it");
    assert_eq!(grant_rows(&pool, ctx, to).await, 0);
}

// ── Connections ───────────────────────────────────────────────────────────────────────────────────

/// A teamless connection is admin-only to provision; a demoted admin creates no ingress identity.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn provision_connection(pool: PgPool) {
    let admin = an_admin(&pool, "ra-admin").await;
    let door = move |pool: PgPool| async move {
        let by = test_support::human_principal_for(&pool, admin).await;
        connection_service::provision(&pool, &by, &connection_req("ra-conn")).await
    };
    both_races(&pool, admin, forbidden, door).await;
    let made: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_connections")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(made, 0, "a refused provision wrote a connection");
    door(pool.clone()).await.expect("the admin provisions it");
}

/// Every per-row connection mutator, raced in turn on one connection; none changes it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn connection_mutators(pool: PgPool) {
    let admin = an_admin(&pool, "ra-admin").await;
    let c = connection(&pool, admin, "ra-conn").await.id;
    let before = connection_row(&pool, c).await;

    both_races(
        &pool,
        admin,
        connection_refusal,
        move |pool: PgPool| async move {
            let by = test_support::human_principal_for(&pool, admin).await;
            connection_service::set_webhook_events(&pool, &by, c, &["push".to_string()]).await
        },
    )
    .await;
    both_races(
        &pool,
        admin,
        connection_refusal,
        move |pool: PgPool| async move {
            let by = test_support::human_principal_for(&pool, admin).await;
            connection_service::set_tool_manifest(&pool, &by, c, &["read".to_string()]).await
        },
    )
    .await;
    // The broker is shared, so the test can say a refused attach reached no provider: its gate is
    // ahead of the mint.
    let broker = std::sync::Arc::new(FakeBroker::granting(serde_json::json!({})));
    let b2 = broker.clone();
    both_races(&pool, admin, connection_refusal, move |pool: PgPool| {
        let broker = b2.clone();
        async move {
            let by = test_support::human_principal_for(&pool, admin).await;
            connection_service::attach_credential(
                &pool,
                broker.as_ref(),
                &by,
                c,
                &ConnectionCredential {
                    broker: "vercel-connect".into(),
                    connector: "conn_abc123".into(),
                    installation: None,
                },
            )
            .await
        }
    })
    .await;
    assert_eq!(
        broker.mint_calls(),
        0,
        "a refused attach minted at the provider"
    );
    both_races(
        &pool,
        admin,
        connection_refusal,
        move |pool: PgPool| async move {
            let by = test_support::human_principal_for(&pool, admin).await;
            connection_service::revoke(&pool, c, &by).await
        },
    )
    .await;
    assert_eq!(
        connection_row(&pool, c).await,
        before,
        "a refused mutator changed the row"
    );

    let by = test_support::human_principal_for(&pool, admin).await;
    connection_service::revoke(&pool, c, &by)
        .await
        .expect("the admin revokes it");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn connection_reach(pool: PgPool) {
    let admin = an_admin(&pool, "ra-admin").await;
    let c = connection(&pool, admin, "ra-conn").await.id;
    let to = team(&pool, "ra-recipient").await;

    both_races(
        &pool,
        admin,
        connection_refusal,
        move |pool: PgPool| async move {
            let by = test_support::human_principal_for(&pool, admin).await;
            connection_service::grant_reach(&pool, &by, c, to, None).await
        },
    )
    .await;
    assert_eq!(
        grant_rows(&pool, c, to).await,
        0,
        "a refused grant_reach wrote a grant"
    );

    let by = test_support::human_principal_for(&pool, admin).await;
    connection_service::grant_reach(&pool, &by, c, to, None)
        .await
        .expect("the admin grants reach");
    both_races(
        &pool,
        admin,
        connection_refusal,
        move |pool: PgPool| async move {
            let by = test_support::human_principal_for(&pool, admin).await;
            connection_service::revoke_reach(&pool, &by, c, to).await
        },
    )
    .await;
    assert_eq!(
        grant_rows(&pool, c, to).await,
        1,
        "a refused revoke_reach deleted the grant"
    );
}

// ── Lock order ────────────────────────────────────────────────────────────────────────────────────

/// A lock-order witness, not a proof: admin A rotates a machine's secret while admin B revokes A
/// (a demotion), from one start line, several times. The rotation takes A's governance row
/// `FOR KEY SHARE`, then the severed personal team and its member row, then the credential row; the
/// revoke takes A's standing row, then the governance pair (A `FOR UPDATE`, B `KEY SHARE`,
/// ascending), then deletes A's row and revokes A's refresh tokens. They contend on A's governance
/// row and on nothing else, so every round ends in one serial order: the rotation first (it hands
/// back a secret, then A is revoked) or the revoke first (the rotation is refused with the
/// missing-id sentence). A deadlock aborts one side and fails the round.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_rotation_and_its_rotators_revoke_never_deadlock(pool: PgPool) {
    let person = profile(&pool, "ra-person").await;
    test_support::approve(&pool, person).await;
    let theirs: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(person)
        .fetch_one(&pool)
        .await
        .unwrap();

    for round in 0..8 {
        let a = an_admin(&pool, &format!("ra-a-{round}")).await;
        let b = an_admin(&pool, &format!("ra-b-{round}")).await;
        let client = issued_machine(&pool, a).await;
        // The person's own machine, so their personal team admits it and the rotation severs it.
        sqlx::query("UPDATE kb_machine_clients SET registered_by_profile_id = $2 WHERE id = $1")
            .bind(client.id)
            .bind(person)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(theirs)
        .bind(client.profile_id)
        .execute(&pool)
        .await
        .unwrap();
        let revoker = test_support::system_admin_proof_for(&pool, b).await;

        let start = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let (p1, s1) = (pool.clone(), start.clone());
        let rotate = tokio::spawn(async move {
            let by = test_support::human_principal_for(&p1, a).await;
            s1.wait().await;
            machine_client_service::rotate_secret(&p1, &by, client.id, 0, AuthMode::TemperAs).await
        });
        let (p2, s2) = (pool.clone(), start.clone());
        let revoke = tokio::spawn(async move {
            s2.wait().await;
            access_service::admin_revoke(&p2, &revoker, ProfileId::from(a), "round".into()).await
        });
        let (rotated, revoked) = (rotate.await.unwrap(), revoke.await.unwrap());
        revoked.unwrap_or_else(|e| panic!("round {round}: the revoke failed: {e:?}"));
        match rotated {
            Ok(_) => {}
            Err(err) if machine_client_refusal(&err) => {}
            Err(other) => panic!("round {round}: not one serial order: {other:?}"),
        }
    }
}
