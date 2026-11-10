#![cfg(feature = "test-db")]
//! Reconcile on an admin-only map (the L0 kernel, or a map bound to temper-system or the gating team)
//! and genesis at a requested id decided their system-admin arm on a pool read and never asked
//! again. Each now asks again as the first statement of its SERIALIZABLE transaction, after
//! `SET TRANSACTION`, under the governance lock (`auth::is_system_admin_locked`, held to commit).
//!
//! **In flight:** the admin's demotion is left open, the act is started, and the test waits until it
//! is blocked at `… kb_principal_governance … FOR KEY SHARE`. The pool reads ahead of it are plain and
//! never wait on an uncommitted delete. The demotion commits; SERIALIZABLE answers the waiting lock
//! with a serialization failure, which the act maps to the `403` a non-admin gets (not the
//! retryable `409` a conflicting reconcile gets at commit), and nothing is written. These halves
//! fail on the code before this change: the act never blocked, and it wrote.
//!
//! **Committed first:** a regression pin. The act's own pool read already sees a committed
//! demotion and answers `403`.
//!
//! **Unchanged:** an admin who is not demoted reconciles L0 and places a map at a requested id as
//! before, and one whose demotion rolls back proceeds once the wait ends.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::reconcile::{CreateCogmapRequest, ReconcileCogmapRequest, ReconcileEntry};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CreateCognitiveMap, ReconcileCognitiveMap, Surface};

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

const GOVERNANCE_RECHECK: &str = "%FROM kb_principal_governance%FOR KEY SHARE%";

/// The production-shaped config: the gating team is the root team the L0 migration bore. With it
/// unset, `is_system_admin` is false for everyone and L0 is immutable.
async fn set_gating_team(pool: &PgPool) {
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1")
        .execute(pool)
        .await
        .expect("set gating team slug");
}

async fn an_admin(pool: &PgPool, email: &str) -> Uuid {
    let id = common::fixtures::create_test_profile(pool, email).await;
    common::fixtures::make_test_admin(pool, id).await;
    id
}

fn as_profile(pool: &PgPool, id: Uuid) -> DbBackend {
    DbBackend::new(pool.clone(), ProfileId::from(id))
}

/// One pre-embedded kernel landmark (the reconcile suite's shape: a single synthetic chunk, no ONNX).
fn landmark(id: Uuid) -> ReconcileEntry {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: "a kernel landmark".to_string(),
        content_hash: format!("{:0>64}", "ra"),
        embedding: vec![0.1; 768],
        embedded_with: None,
    };
    let content_hash = temper_substrate::content::body_hash_from_chunk_hashes(
        std::slice::from_ref(&chunk.content_hash),
    );
    ReconcileEntry {
        id,
        origin_uri: format!("temper://kernel/ra/{id}"),
        title: "ra landmark".to_string(),
        doc_type: "kernel_landmark".to_string(),
        content_hash,
        chunks_packed: pack_chunks(std::slice::from_ref(&chunk)).expect("pack"),
        facets: serde_json::json!({}),
        edges: vec![],
    }
}

fn reconcile(entry: Uuid) -> ReconcileCognitiveMap {
    ReconcileCognitiveMap {
        cogmap_id: CogmapId::from(L0_COGMAP),
        request: ReconcileCogmapRequest {
            entries: vec![landmark(entry)],
            fold_resources: vec![],
            fold_edges: vec![],
            telos: None,
        },
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

fn genesis_at(cogmap: Uuid) -> CreateCognitiveMap {
    CreateCognitiveMap {
        request: CreateCogmapRequest {
            cogmap_id: Some(cogmap),
            telos_resource_id: None,
            name: "ra-map".to_string(),
            telos_title: "ra telos".to_string(),
            telos: None,
        },
        origin: Surface::ApiHttp,
    }
}

async fn resource_exists(pool: &PgPool, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM kb_resources WHERE id = $1)")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn cogmap_exists(pool: &PgPool, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM kb_cogmaps WHERE id = $1)")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
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
            "the act never blocked at the governance re-check"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// An admin demoted while their reconcile of the L0 kernel is in flight is refused with `403`, and
/// the kernel is unchanged. Committed first, the pool's own read refuses.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reconcile_of_the_kernel(pool: PgPool) {
    set_gating_team(&pool).await;
    let admin = an_admin(&pool, "ra-admin@example.com").await;

    let entry = Uuid::now_v7();
    let (holder, pid) = demotion_in_flight(&pool, admin).await;
    let p = pool.clone();
    let contender = tokio::spawn(async move {
        as_profile(&p, admin)
            .reconcile_cognitive_map(reconcile(entry))
            .await
    });
    await_blocked_at_the_recheck(&pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, TemperError::Forbidden), "in flight: {err:?}");
    assert!(
        !resource_exists(&pool, entry).await,
        "a refused reconcile wrote the kernel"
    );

    let err = as_profile(&pool, admin)
        .reconcile_cognitive_map(reconcile(entry))
        .await
        .unwrap_err();
    assert!(
        matches!(err, TemperError::Forbidden),
        "committed first: {err:?}"
    );
    assert!(!resource_exists(&pool, entry).await);
}

/// An admin demoted while their genesis at a requested id is in flight is refused with `403`, and no
/// map is placed there. Committed first, the pool's read no longer honors the id at all: the
/// genesis mints fresh ids, and nothing lands at the requested one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn genesis_at_a_requested_id(pool: PgPool) {
    set_gating_team(&pool).await;
    let admin = an_admin(&pool, "ra-admin@example.com").await;

    let at = Uuid::now_v7();
    let (holder, pid) = demotion_in_flight(&pool, admin).await;
    let p = pool.clone();
    let contender = tokio::spawn(async move {
        as_profile(&p, admin)
            .create_cognitive_map(genesis_at(at))
            .await
    });
    await_blocked_at_the_recheck(&pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, TemperError::Forbidden), "in flight: {err:?}");
    assert!(
        !cogmap_exists(&pool, at).await,
        "a refused genesis placed a map at the requested id"
    );

    let fresh = as_profile(&pool, admin)
        .create_cognitive_map(genesis_at(at))
        .await
        .expect("a non-admin's genesis is legal at fresh ids")
        .value;
    assert_ne!(
        fresh.cogmap_id, at,
        "committed first: the requested id was honored"
    );
    assert!(!cogmap_exists(&pool, at).await);
}

/// An admin who is not demoted is unchanged: they reconcile the kernel and place a map at a requested
/// id, and another admin's demotion in flight does not hold them up (the re-check locks only the
/// caller's own governance row).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_admin_who_is_not_demoted_is_unchanged(pool: PgPool) {
    set_gating_team(&pool).await;
    let admin = an_admin(&pool, "ra-admin@example.com").await;
    let other = an_admin(&pool, "ra-other@example.com").await;
    let (holder, _) = demotion_in_flight(&pool, other).await;

    let entry = Uuid::now_v7();
    as_profile(&pool, admin)
        .reconcile_cognitive_map(reconcile(entry))
        .await
        .expect("the admin reconciles the kernel");
    assert!(
        resource_exists(&pool, entry).await,
        "the landmark was written"
    );

    let at = Uuid::now_v7();
    let placed = as_profile(&pool, admin)
        .create_cognitive_map(genesis_at(at))
        .await
        .expect("the admin places a map at a requested id")
        .value;
    assert_eq!(placed.cogmap_id, at);
    assert!(placed.created);
    holder.rollback().await.unwrap();
}

/// A demotion that rolls back changes nothing: the reconcile that waited on it at the re-check
/// proceeds and writes. Pins that the wait is not itself a refusal.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_demotion_that_rolls_back_lets_the_reconcile_proceed(pool: PgPool) {
    set_gating_team(&pool).await;
    let admin = an_admin(&pool, "ra-admin@example.com").await;

    let entry = Uuid::now_v7();
    let (holder, pid) = demotion_in_flight(&pool, admin).await;
    let p = pool.clone();
    let contender = tokio::spawn(async move {
        as_profile(&p, admin)
            .reconcile_cognitive_map(reconcile(entry))
            .await
    });
    await_blocked_at_the_recheck(&pool, pid).await;
    holder.rollback().await.unwrap();
    contender
        .await
        .unwrap()
        .expect("the admin, still an admin, reconciles the kernel");
    assert!(resource_exists(&pool, entry).await);
}
