#![cfg(feature = "test-db")]
//! Two updates of one resource never deadlock (task 01a0fd62-bb17-7442-8b01-11b2c6e01319).
//!
//! `update_resource_in_tx` fires its events in a fixed order: body, then property unsets, then
//! property sets, then the title. The row each event's projector locks differs by event: a body
//! write reaches `_recompute_resource_body_hash`, which takes the `kb_resources` row
//! `FOR NO KEY UPDATE`; a property set or unset folds the key's live `kb_properties` rows; a
//! title writes the `kb_resources` row. So which row an update locks first depends on what it
//! carries, and two updates of one resource can each hold the row the other wants next.
//!
//! Each witness reproduces one cycle with two transactions. Transaction A splits one update into
//! two calls, so it can stop between the events of a single update (the point where a real
//! update holds its first row and has not yet asked for its second). Transaction B is one whole
//! update, run while A holds its first row. When B is waiting on a lock, A makes its second call.
//! Both must commit; a deadlock aborts one with `40P01`.
//!
//! The two cycles:
//!   * **resource row against a property row**: A holds `note` and then retitles; B carries a
//!     body and `note`, so it takes the resource row (body hash) before `note`.
//!   * **two property rows**: A unsets `x` and then sets `y`; B unsets `y` and then sets `x`. No
//!     resource row is involved. The door reaches this order from one PATCH, because it fires
//!     every unset (an `open_meta` null) before every set; `serde_json` maps without
//!     `preserve_order` only sort keys WITHIN each half.

mod common;

use std::time::Duration;

use sha2::Digest;
use sqlx::PgPool;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams, UpdateParams};
use uuid::Uuid;

// Local fixture helpers, duplicated per file rather than shared (replay_roundtrip.rs's header
// note).

fn chunk(prose: &str) -> IncomingChunk {
    IncomingChunk {
        chunk_index: 0,
        content_hash: format!("{:x}", sha2::Sha256::digest(prose.trim())),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: String::new(),
        heading_depth: 0,
    }
}

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

/// A resource with a body and a live row for each of `note`, `x` and `y`, so a set or an unset of
/// any of them folds (and so locks) an existing row.
async fn seed(pool: &PgPool) -> (ResourceId, EntityId) {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), "locks", "locks")
            .await
            .unwrap(),
    );
    let properties = [
        ("note".to_owned(), serde_json::json!("n0")),
        ("x".to_owned(), serde_json::json!("x0")),
        ("y".to_owned(), serde_json::json!("y0")),
    ];
    let resource = writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: "lock order",
            origin_uri: "test://lock-order",
            body: "the first body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &properties,
            chunks: Some(vec![chunk("the first body")]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("seed resource through the create path");
    (resource, emitter)
}

fn params<'a>(
    resource: ResourceId,
    emitter: EntityId,
    body: Option<&'a str>,
    title: Option<&'a str>,
    properties: &'a [(String, serde_json::Value)],
    unset_keys: &'a [String],
) -> UpdateParams<'a> {
    UpdateParams {
        resource,
        body,
        title,
        origin_uri: None,
        properties,
        unset_keys,
        chunks: body.map(|b| vec![chunk(b)]),
        sources: vec![],
        content_block: None,
        rehome_to: None,
        emitter,
    }
}

/// Run transaction B (one whole update, committed) on its own connection. Returns B's backend pid
/// once it is known, and the task that finishes B.
async fn spawn_b(
    pool: &PgPool,
    resource: ResourceId,
    emitter: EntityId,
    body: Option<&'static str>,
    properties: Vec<(String, serde_json::Value)>,
    unset_keys: Vec<String>,
) -> (i32, tokio::task::JoinHandle<Result<(), String>>) {
    let pool = pool.clone();
    let (pid_tx, pid_rx) = tokio::sync::oneshot::channel();
    let b = tokio::spawn(async move {
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        pid_tx.send(pid).unwrap();
        writes::update_resource_in_tx(
            &mut tx,
            params(resource, emitter, body, None, &properties, &unset_keys),
            EventContext::default(),
            false,
        )
        .await
        .map_err(|e| format!("{e:#}"))?;
        tx.commit().await.map_err(|e| e.to_string())
    });
    (pid_rx.await.unwrap(), b)
}

/// Wait until backend `pid` is blocked on a lock.
async fn await_lock_wait(pool: &PgPool, pid: i32) {
    for _ in 0..200 {
        let waiting: Option<String> =
            sqlx::query_scalar("SELECT wait_event_type FROM pg_stat_activity WHERE pid = $1")
                .bind(pid)
                .fetch_optional(pool)
                .await
                .unwrap()
                .flatten();
        if waiting.as_deref() == Some("Lock") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("transaction B never waited on a lock: A held nothing B needed");
}

/// Both transactions must finish, and neither may be the deadlock victim.
async fn assert_both_commit(a: Result<(), String>, b: tokio::task::JoinHandle<Result<(), String>>) {
    let b = tokio::time::timeout(Duration::from_secs(30), b)
        .await
        .expect("transaction B never finished")
        .expect("transaction B panicked");
    assert!(a.is_ok(), "transaction A failed: {a:?}");
    assert!(b.is_ok(), "transaction B failed: {b:?}");
}

/// **The resource row against a property row.** A holds `note`, then retitles. B, a body + `note`
/// update, takes the resource row for the body hash and then waits on `note`. A's retitle then
/// waits on the resource row B holds.
///
/// FAILS IF an update can lock a property row before the resource row: Postgres aborts A or B
/// with `40P01` (deadlock detected).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_title_update_and_a_body_update_of_one_resource_both_commit(pool: PgPool) {
    let (resource, emitter) = seed(&pool).await;

    let note_a = [("note".to_owned(), serde_json::json!("from A"))];
    let mut a = pool.begin().await.unwrap();
    writes::update_resource_in_tx(
        &mut a,
        params(resource, emitter, None, None, &note_a, &[]),
        EventContext::default(),
        false,
    )
    .await
    .expect("A's property set");

    let (b_pid, b) = spawn_b(
        &pool,
        resource,
        emitter,
        Some("the second body, from B"),
        vec![("note".to_owned(), serde_json::json!("from B"))],
        vec![],
    )
    .await;
    await_lock_wait(&pool, b_pid).await;

    let a_result = async {
        writes::update_resource_in_tx(
            &mut a,
            params(resource, emitter, None, Some("retitled by A"), &[], &[]),
            EventContext::default(),
            false,
        )
        .await
        .map_err(|e| format!("{e:#}"))?;
        a.commit().await.map_err(|e| e.to_string())
    }
    .await;
    assert_both_commit(a_result, b).await;
}

/// **Two property rows.** A unsets `x`, then sets `y`. B unsets `y`, then sets `x`: one PATCH
/// with `{"x": .., "y": null}`, since the door fires every unset before every set.
///
/// FAILS IF two updates can lock one resource's property rows in different orders: Postgres
/// aborts A or B with `40P01`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unset_and_a_set_of_two_keys_in_opposite_orders_both_commit(pool: PgPool) {
    let (resource, emitter) = seed(&pool).await;

    let unset_x = ["x".to_owned()];
    let mut a = pool.begin().await.unwrap();
    writes::update_resource_in_tx(
        &mut a,
        params(resource, emitter, None, None, &[], &unset_x),
        EventContext::default(),
        false,
    )
    .await
    .expect("A's unset of x");

    let (b_pid, b) = spawn_b(
        &pool,
        resource,
        emitter,
        None,
        vec![("x".to_owned(), serde_json::json!("x from B"))],
        vec!["y".to_owned()],
    )
    .await;
    await_lock_wait(&pool, b_pid).await;

    let set_y = [("y".to_owned(), serde_json::json!("y from A"))];
    let a_result = async {
        writes::update_resource_in_tx(
            &mut a,
            params(resource, emitter, None, None, &set_y, &[]),
            EventContext::default(),
            false,
        )
        .await
        .map_err(|e| format!("{e:#}"))?;
        a.commit().await.map_err(|e| e.to_string())
    }
    .await;
    assert_both_commit(a_result, b).await;
}
