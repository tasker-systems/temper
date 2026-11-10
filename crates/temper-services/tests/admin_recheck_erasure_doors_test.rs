#![cfg(feature = "test-db")]
//! The erasure doors take a `&SystemAdmin` proof minted on the pool and, before this change, never
//! asked again: an admin demoted after the mint, or while the act was in flight, still erased a
//! resource, scrubbed a field or a block's history, erased a person, or set or removed a retention
//! policy (which decides what every later erasure keeps). Each now asks again on the act's own
//! transaction, under the governance lock (`resource_erasure_service::operator_holds`, over
//! `auth::is_system_admin_locked`), and a demoted admin is told what a non-admin is told at these
//! doors: `404 "not found"`.
//!
//! **In flight:** the admin's demotion is left open, the door is started, and the test waits until
//! it is blocked at `… kb_principal_governance … FOR KEY SHARE`. For resource erasure and the two
//! scrubs that block is the act's transaction: their entry gate is a plain read, which passes an
//! uncommitted demotion, so the in-flight half proves the lock that holds to the commit. For person
//! erasure and the retention policies it is the first lock they take (see each test). The demotion
//! commits and the door is refused, having written nothing.
//!
//! **Committed first:** the proof was minted before the demotion; the door is refused before it
//! validates anything (the entry gate), so no refusal is recorded in the demoted admin's name.
//!
//! Both halves of every door test fail on the code before this change: the door never blocked,
//! and the stale proof erased. `a_stale_proof_learns_nothing_about_the_request` pins the entry
//! gate's place ahead of validation. The last test is a lock-order witness and passes on that code
//! too, which took no governance lock to contend on.
//!
//! Person erasure runs its Slack pass under its entry gate's lock; with no Slack configured here,
//! no test shows a Slack revoke withheld, only that the act is refused and the estate intact.

use std::future::Future;

use sqlx::PgPool;
use temper_core::types::authorship::ActContext;
use temper_core::types::erasure::{RetentionCategory, RetentionPolicySet, ScrubFieldKind};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_services::auth::SystemAdmin;
use temper_services::backend::DbBackend;
use temper_services::error::ApiError;
use temper_services::services::block_history_scrub_service::{
    execute_block_history_scrub, BlockHistoryScrubRequest,
};
use temper_services::services::erasure_retention_service;
use temper_services::services::erasure_service::{execute_erasure, ErasureEnv};
use temper_services::services::field_scrub_service::{execute_field_scrub, FieldScrubRequest};
use temper_services::services::resource_erasure_service::{
    execute_resource_erasure, ResourceErasureRequest,
};
use temper_services::test_support;
use temper_workflow::operations::{Backend, BodyUpdate, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;
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

/// A person with a resource in a context of their own; the resource carries a body, so blocks.
async fn person_with_a_resource(pool: &PgPool, handle: &str) -> (Uuid, ResourceId) {
    let person = profile(pool, handle).await;
    test_support::approve(pool, person).await;
    let context: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, 'ra-ctx', 'ra-ctx') RETURNING id",
    )
    .bind(person)
    .fetch_one(pool)
    .await
    .expect("context");
    let created = DbBackend::new(pool.clone(), ProfileId::from(person))
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: "ra-doomed".into(),
            doctype: "research".into(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "a title worth scrubbing".into(),
            body: Some(BodyUpdate {
                content: "A paragraph of body text.".into(),
                content_hash: None,
                chunks_packed: None,
                sources: vec![],
                content_block: None,
            }),
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some("test://ra".into()),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create resource")
        .value;
    (person, created.id)
}

async fn an_admin(pool: &PgPool) -> Uuid {
    let id = profile(pool, "ra-operator").await;
    test_support::approved_admin(pool, id).await;
    id
}

async fn proof(pool: &PgPool, admin: Uuid) -> SystemAdmin {
    test_support::system_admin_proof_for(pool, admin).await
}

/// Every erasure-family event on the ledger: completions and recorded refusals alike.
async fn erasure_events(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name ~ '(erase|eras|scrub)'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn erased(pool: &PgPool, resource: ResourceId) -> bool {
    sqlx::query_scalar("SELECT erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
        .bind(Uuid::from(resource))
        .fetch_one(pool)
        .await
        .unwrap()
}

// ── The two races ─────────────────────────────────────────────────────────────────────────────────

const GOVERNANCE_RECHECK: &str = "%FROM kb_principal_governance%FOR KEY SHARE%";

async fn demotion_in_flight(
    pool: &PgPool,
    who: Uuid,
) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut holder = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    let _: bool = sqlx::query_scalar("SELECT principal_governance_set($1, false, NULL, 'test')")
        .bind(who)
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    (holder, pid)
}

async fn await_blocked_at_the_recheck(pool: &PgPool, holder: i32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)) AND query ILIKE $2)",
        )
        .bind(holder)
        .bind(GOVERNANCE_RECHECK)
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the door never blocked at the governance re-check"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

fn the_erasure_refusal(err: &ApiError) -> bool {
    matches!(err, ApiError::NotFound(why) if why == "not found")
}

/// Both races on `door`, each with a proof minted before the demotion. `admin` is an admin again
/// afterwards.
async fn both_races<T, F, Fut>(pool: &PgPool, admin: Uuid, door: F)
where
    T: std::fmt::Debug + Send + 'static,
    F: Fn(PgPool, SystemAdmin) -> Fut,
    Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    let stale = proof(pool, admin).await;
    let (holder, pid) = demotion_in_flight(pool, admin).await;
    let contender = tokio::spawn(door(pool.clone(), stale));
    await_blocked_at_the_recheck(pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(the_erasure_refusal(&err), "in flight: {err:?}");
    test_support::grant_governance(pool, admin).await;

    let stale = proof(pool, admin).await;
    let (holder, _) = demotion_in_flight(pool, admin).await;
    holder.commit().await.unwrap();
    let err = door(pool.clone(), stale).await.unwrap_err();
    assert!(the_erasure_refusal(&err), "committed first: {err:?}");
    test_support::grant_governance(pool, admin).await;
}

// ── The doors ─────────────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn resource_erasure(pool: PgPool) {
    let admin = an_admin(&pool).await;
    let (_, resource) = person_with_a_resource(&pool, "ra-person").await;
    let before = erasure_events(&pool).await;
    let door = move |pool: PgPool, admin: SystemAdmin| async move {
        execute_resource_erasure(
            &pool,
            None,
            &admin,
            ResourceErasureRequest {
                resource,
                also_strike_blobs: &[],
                surface: Surface::ApiHttp,
            },
        )
        .await
    };
    both_races(&pool, admin, door).await;
    assert!(!erased(&pool, resource).await, "a refused erasure erased");
    assert_eq!(
        erasure_events(&pool).await,
        before,
        "a refused erasure recorded an event"
    );
    door(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin erases it");
    assert!(erased(&pool, resource).await);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn field_scrub(pool: PgPool) {
    let admin = an_admin(&pool).await;
    let (_, resource) = person_with_a_resource(&pool, "ra-person").await;
    let before = erasure_events(&pool).await;
    let door = move |pool: PgPool, admin: SystemAdmin| async move {
        execute_field_scrub(
            &pool,
            &admin,
            FieldScrubRequest {
                resource,
                field: ScrubFieldKind::Title,
                family: None,
                clear: true,
                surface: Surface::ApiHttp,
            },
        )
        .await
    };
    both_races(&pool, admin, door).await;
    assert_eq!(
        erasure_events(&pool).await,
        before,
        "a refused scrub recorded an event"
    );
    let title: String = sqlx::query_scalar("SELECT title FROM kb_resources WHERE id = $1")
        .bind(Uuid::from(resource))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        title, "a title worth scrubbing",
        "a refused scrub cleared the title"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn block_history_scrub(pool: PgPool) {
    let admin = an_admin(&pool).await;
    let (_, resource) = person_with_a_resource(&pool, "ra-person").await;
    let blocks: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM kb_content_blocks WHERE resource_id = $1 ORDER BY seq")
            .bind(Uuid::from(resource))
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!blocks.is_empty(), "the fixture's resource has blocks");
    let before = erasure_events(&pool).await;
    let door = move |pool: PgPool, admin: SystemAdmin| {
        let blocks = blocks.clone();
        async move {
            execute_block_history_scrub(
                &pool,
                &admin,
                BlockHistoryScrubRequest {
                    resource,
                    blocks: &blocks,
                    surface: Surface::ApiHttp,
                },
            )
            .await
        }
    };
    both_races(&pool, admin, door).await;
    assert_eq!(
        erasure_events(&pool).await,
        before,
        "a refused scrub recorded an event"
    );
}

/// Person erasure: the first lock it takes is its entry gate's, a locked read, ahead of the Slack
/// revoke, so an in-flight demotion stops it before anything reaches Slack. Its act transaction
/// asks again (the check that holds to the commit); that second check is the backstop for a
/// demotion committing while Slack is called and is not separately raced here.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn person_erasure(pool: PgPool) {
    let admin = an_admin(&pool).await;
    let (person, resource) = person_with_a_resource(&pool, "ra-person").await;
    let before = erasure_events(&pool).await;
    let door = move |pool: PgPool, admin: SystemAdmin| async move {
        execute_erasure(
            &pool,
            &admin,
            ProfileId::from(person),
            Uuid::now_v7(),
            Surface::ApiHttp,
            &ErasureEnv {
                slack: None,
                auth_provider: None,
            },
        )
        .await
    };
    both_races(&pool, admin, door).await;
    assert!(
        !erased(&pool, resource).await,
        "a refused person erasure erased the estate"
    );
    assert_eq!(erasure_events(&pool).await, before);
    let tombstoned: bool =
        sqlx::query_scalar("SELECT tombstoned_at IS NOT NULL FROM kb_profiles WHERE id = $1")
            .bind(person)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !tombstoned,
        "a refused person erasure tombstoned the person"
    );
}

/// A retention policy decides what every later erasure keeps. Set and remove each lock governance
/// first on their write's transaction.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn retention_policies(pool: PgPool) {
    let admin = an_admin(&pool).await;
    let policies = |pool: PgPool| async move {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kb_erasure_retention_policies")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    let set = |pool: PgPool, admin: SystemAdmin| async move {
        erasure_retention_service::set(
            &pool,
            &admin,
            &RetentionPolicySet {
                name: "ra-keep".into(),
                category: RetentionCategory::StandingReasons,
                basis: "audit".into(),
                period: "1 year".into(),
            },
        )
        .await
    };
    both_races(&pool, admin, set).await;
    assert_eq!(
        policies(pool.clone()).await,
        0,
        "a refused set wrote a policy"
    );

    set(pool.clone(), proof(&pool, admin).await)
        .await
        .expect("the admin sets it");
    let remove = |pool: PgPool, admin: SystemAdmin| async move {
        erasure_retention_service::remove(&pool, &admin, RetentionCategory::StandingReasons).await
    };
    both_races(&pool, admin, remove).await;
    assert_eq!(
        policies(pool.clone()).await,
        1,
        "a refused remove deleted the policy"
    );
}

/// A demoted admin holding a stale proof is refused by the entry gate before the request is judged:
/// a missing resource and a list naming a blob or block twice each answer the doors' `404 "not
/// found"`, not the `404` or `400` an admin gets for them. Fails on the code before this change.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_stale_proof_learns_nothing_about_the_request(pool: PgPool) {
    use temper_core::types::ids::BlobId;
    let admin = an_admin(&pool).await;
    let (_, resource) = person_with_a_resource(&pool, "ra-person").await;
    let stale = proof(&pool, admin).await;
    let (holder, _) = demotion_in_flight(&pool, admin).await;
    holder.commit().await.unwrap();

    let missing = ResourceId::from(Uuid::now_v7());
    let blob = BlobId::from(Uuid::now_v7());
    let err = execute_resource_erasure(
        &pool,
        None,
        &stale,
        ResourceErasureRequest {
            resource: missing,
            also_strike_blobs: &[],
            surface: Surface::ApiHttp,
        },
    )
    .await
    .unwrap_err();
    assert!(the_erasure_refusal(&err), "a missing resource: {err:?}");
    let err = execute_resource_erasure(
        &pool,
        None,
        &stale,
        ResourceErasureRequest {
            resource,
            also_strike_blobs: &[blob, blob],
            surface: Surface::ApiHttp,
        },
    )
    .await
    .unwrap_err();
    assert!(the_erasure_refusal(&err), "a blob listed twice: {err:?}");
    let block = Uuid::now_v7();
    let err = execute_block_history_scrub(
        &pool,
        &stale,
        BlockHistoryScrubRequest {
            resource,
            blocks: &[block, block],
            surface: Surface::ApiHttp,
        },
    )
    .await
    .unwrap_err();
    assert!(the_erasure_refusal(&err), "a block listed twice: {err:?}");
    let err = execute_field_scrub(
        &pool,
        &stale,
        FieldScrubRequest {
            resource: missing,
            field: ScrubFieldKind::Title,
            family: None,
            clear: true,
            surface: Surface::ApiHttp,
        },
    )
    .await
    .unwrap_err();
    assert!(
        the_erasure_refusal(&err),
        "a missing resource to scrub: {err:?}"
    );
}

// ── Lock order ────────────────────────────────────────────────────────────────────────────────────

/// A lock-order witness, not a proof: admin X erases a person while admin Y revokes X, from one
/// start line, several times. The erasure takes X's governance row `FOR KEY SHARE` (its gate, then
/// its act), then the act's advisory and resource locks; the revoke takes X's standing row, then the
/// governance pair (X `FOR UPDATE`, Y `KEY SHARE`, ascending), then deletes X's row and revokes X's
/// refresh tokens. Neither takes a row of the other's after its first lock, so every round ends in
/// one serial order: the erasure first (it completes, then the revoke lands), or the revoke first
/// (the erasure is refused, the person intact). A deadlock aborts one side and fails the round.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_person_erasure_and_its_operators_revoke_never_deadlock(pool: PgPool) {
    use temper_services::services::access_service;
    for round in 0..6 {
        let x = profile(&pool, &format!("ra-x-{round}")).await;
        test_support::approved_admin(&pool, x).await;
        let y = profile(&pool, &format!("ra-y-{round}")).await;
        test_support::approved_admin(&pool, y).await;
        let (person, _) = person_with_a_resource(&pool, &format!("ra-p-{round}")).await;
        let (px, py) = (proof(&pool, x).await, proof(&pool, y).await);

        let start = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let (p1, s1) = (pool.clone(), start.clone());
        let erase = tokio::spawn(async move {
            s1.wait().await;
            execute_erasure(
                &p1,
                &px,
                ProfileId::from(person),
                Uuid::now_v7(),
                Surface::ApiHttp,
                &ErasureEnv {
                    slack: None,
                    auth_provider: None,
                },
            )
            .await
        });
        let (p2, s2) = (pool.clone(), start.clone());
        let revoke = tokio::spawn(async move {
            s2.wait().await;
            access_service::admin_revoke(&p2, &py, ProfileId::from(x), "round".into()).await
        });
        let (erased_person, revoked) = (erase.await.unwrap(), revoke.await.unwrap());
        revoked.unwrap_or_else(|e| panic!("round {round}: the revoke failed: {e:?}"));
        let tombstoned: bool =
            sqlx::query_scalar("SELECT tombstoned_at IS NOT NULL FROM kb_profiles WHERE id = $1")
                .bind(person)
                .fetch_one(&pool)
                .await
                .unwrap();
        match erased_person {
            Ok(_) => assert!(
                tombstoned,
                "round {round}: completed, yet the person stands"
            ),
            Err(err) if the_erasure_refusal(&err) => {
                assert!(
                    !tombstoned,
                    "round {round}: refused, yet the person is erased"
                )
            }
            Err(other) => panic!("round {round}: not one serial order: {other:?}"),
        }
    }
}
