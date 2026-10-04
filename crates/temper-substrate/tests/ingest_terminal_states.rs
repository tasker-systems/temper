#![cfg(feature = "test-db")]
//! Terminal ingest states (resource erasure build order 2e, migration 20261003000110).
//!
//! `cancelled` and `abandoned` are terminal: `block_append` and `resource_finalize` refuse them with
//! SQLSTATE `TF004`, and the Rust guards over an incomplete body (`update_resource`'s whole-body
//! arm, the re-block classification) treat every state that is not `complete` as incomplete. The
//! block history scrub sets `cancelled` (`_block_history_scrub_apply`, migration 20261003000210);
//! `abandoned` is reserved for an abandoned-ingest reaper, and nothing sets it yet. These tests set
//! the state directly, to isolate the guards from the act that ends an ingest. The ingest itself is
//! begun through the real segmented write path.
//!
//! Witnesses:
//!   * for each terminal state: an append of a NEW seq, a re-append of an already-landed seq (the
//!     refusal precedes the idempotency short-circuit) and a finalize whose counts and hash are
//!     correct are each refused with `TF004`, and `ingest_state` is unchanged afterwards;
//!   * the race: a transaction holding the row `FOR UPDATE` while it sets `cancelled` (standing in
//!     for the scrub) makes an append wait; once it commits the append refuses with `TF004` — the
//!     `FOR KEY SHARE` state read re-reads the committed row;
//!   * `update_resource` bails on a whole-body write over an ended ingest;
//!   * the re-block declines an ended ingest as `IngestEnded`.

mod common;

use sha2::Digest;
use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_substrate::content::{prepare_block_from_chunks, IncomingChunk, PreparedBlock};
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::writes::{
    self, AppendParams, CreateMode, CreateParams, FinalizeParams, ReblockDeclineKind,
    ReblockOutcome, ReblockParams, UpdateParams,
};
use uuid::Uuid;

/// The terminal states the CHECK admits beside `in_progress` and `complete`.
const TERMINAL: [&str; 2] = ["cancelled", "abandoned"];

fn chunk_hash(prose: &str) -> String {
    format!("{:x}", sha2::Sha256::digest(prose.trim()))
}

fn chunk(prose: &str, header: &str) -> IncomingChunk {
    IncomingChunk {
        chunk_index: 0,
        content_hash: chunk_hash(prose),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: header.to_string(),
        heading_depth: if header.is_empty() { 0 } else { 1 },
    }
}

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: EntityId =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), entity)
}

async fn make_home(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

/// Re-register `resource_finalized`: migration 20260708000012 inserts it; `reset_schema` truncates it.
async fn register_resource_finalized(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO kb_event_types (name, payload_schema, schema_version, category) \
         VALUES ('resource_finalized', NULL, 1, 'domain') \
         ON CONFLICT (name) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("re-register resource_finalized");
}

/// The SQLSTATE of the database error under a substrate write's `anyhow` chain, if any.
fn sqlstate(e: &anyhow::Error) -> Option<String> {
    e.chain()
        .find_map(|cause| cause.downcast_ref::<sqlx::Error>())
        .and_then(|sqlx_err| sqlx_err.as_database_error())
        .and_then(|db| db.code())
        .map(|code| code.into_owned())
}

async fn ingest_state(pool: &PgPool, resource: ResourceId) -> String {
    sqlx::query_scalar("SELECT ingest_state FROM kb_resources WHERE id = $1")
        .bind(resource.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn live_blocks(pool: &PgPool, resource: ResourceId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Set the ingest state directly, isolating the guard from the act that ends an ingest (the scrub
/// sets `cancelled`; nothing sets `abandoned`).
async fn end_ingest(pool: &PgPool, resource: ResourceId, state: &str) {
    sqlx::query("UPDATE kb_resources SET ingest_state = $2 WHERE id = $1")
        .bind(resource.uuid())
        .bind(state)
        .execute(pool)
        .await
        .expect("the CHECK admits the terminal state");
}

fn segment(seq: i32, prose: &str) -> PreparedBlock {
    prepare_block_from_chunks(seq, None, vec![chunk(prose, "")])
}

/// An ingest in flight, begun through the real segmented write path: block 0 from the begin, and
/// seq 1 appended while the ingest is `in_progress` (which also shows the append path is open
/// before the state ends, so a later refusal is the state's doing).
struct Ingest {
    resource: ResourceId,
    emitter: EntityId,
    landed: PreparedBlock,
}

async fn begin_ingest(pool: &PgPool, slug: &str) -> Ingest {
    let (owner, emitter) = system_actor(pool).await;
    let home = make_home(pool, owner, slug).await;
    let resource = writes::create_resource_with_mode(
        pool,
        CreateParams {
            idempotency_key: None,
            title: slug,
            origin_uri: "test://ingest-terminal-states",
            body: "block zero",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk("block zero", "")]),
            sources: vec![],
        },
        EventContext::default(),
        CreateMode {
            defer: false,
            segmented: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        ingest_state(pool, resource).await,
        "in_progress",
        "the witness needs an ingest actually in flight"
    );
    let landed = segment(1, "segment one");
    writes::append_block(
        pool,
        AppendParams {
            resource,
            block: &landed,
            sources: vec![],
            emitter,
        },
    )
    .await
    .expect("an in-progress ingest accepts an append");
    assert_eq!(live_blocks(pool, resource).await, 2);
    Ingest {
        resource,
        emitter,
        landed,
    }
}

/// For one terminal state: a new-seq append, a re-append of a landed seq and a finalize that would
/// otherwise succeed are each refused with `TF004`, and nothing changes.
async fn assert_terminal_ingest_refuses(pool: &PgPool, state: &str) {
    register_resource_finalized(pool).await;
    let ingest = begin_ingest(pool, &format!("terminal-{state}")).await;
    let body_hash: String = sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id = $1")
        .bind(ingest.resource.uuid())
        .fetch_one(pool)
        .await
        .unwrap();
    end_ingest(pool, ingest.resource, state).await;

    // A new seq.
    let fresh = segment(2, "segment two");
    let err = writes::append_block(
        pool,
        AppendParams {
            resource: ingest.resource,
            block: &fresh,
            sources: vec![],
            emitter: ingest.emitter,
        },
    )
    .await
    .expect_err("an append to a terminal ingest is refused");
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("TF004"),
        "a new-seq append to a {state} ingest refuses with TF004; got {err:#}"
    );
    assert!(
        format!("{err:#}").contains(&format!("ingest is {state}; it cannot be continued")),
        "the refusal names the state; got {err:#}"
    );

    // A re-append of the landed seq 1: without the refusal ahead of it, the idempotency
    // short-circuit would answer this with the existing block id.
    let err = writes::append_block(
        pool,
        AppendParams {
            resource: ingest.resource,
            block: &ingest.landed,
            sources: vec![],
            emitter: ingest.emitter,
        },
    )
    .await
    .expect_err("a re-append of a landed seq to a terminal ingest is refused");
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("TF004"),
        "a re-append to a {state} ingest refuses with TF004 ahead of the idempotency \
         short-circuit; got {err:#}"
    );

    // A finalize whose block count and body hash are both right — only the state refuses it.
    let err = writes::finalize_ingest(
        pool,
        FinalizeParams {
            resource: ingest.resource,
            expected_blocks: 2,
            expected_body_hash: body_hash,
            expected_content_hash: None,
            emitter: ingest.emitter,
        },
    )
    .await
    .expect_err("a finalize of a terminal ingest is refused");
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("TF004"),
        "a finalize of a {state} ingest refuses with TF004; got {err:#}"
    );

    assert_eq!(
        ingest_state(pool, ingest.resource).await,
        state,
        "the refusals leave the terminal state as it was"
    );
    assert_eq!(
        live_blocks(pool, ingest.resource).await,
        2,
        "no refused append landed a block"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_cancelled_ingest_refuses_append_reappend_and_finalize(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    assert_terminal_ingest_refuses(&pool, "cancelled").await;
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_abandoned_ingest_refuses_append_reappend_and_finalize(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    assert_terminal_ingest_refuses(&pool, "abandoned").await;
}

/// The race: a transaction holding R's row `FOR UPDATE` while it sets `cancelled` (standing in for
/// `block_history_scrub_execute`, which takes the same lock before `_block_history_scrub_apply`
/// sets the state) makes an append wait — a 2-second timeout that EXPIRES
/// is the signal — and once it commits the append refuses with `TF004`. The `blob_byte_window_test`
/// choreography the act's witness 20 uses.
///
/// FAILS IF `block_append` reads `ingest_state` without `FOR KEY SHARE`: the read sees the
/// pre-cancel snapshot and passes, the append's foreign-key check then waits on the held row (so
/// the timeout still expires), and once the cancel commits the block LANDS — `expect_err` fails.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_append_racing_the_cancel_waits_then_refuses(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let ingest = begin_ingest(&pool, "race-cancel").await;

    // The stand-in scrub: R's row FOR UPDATE, the state set, NOT committed.
    let mut scrub = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR UPDATE")
        .bind(ingest.resource.uuid())
        .execute(&mut *scrub)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_resources SET ingest_state = 'cancelled' WHERE id = $1")
        .bind(ingest.resource.uuid())
        .execute(&mut *scrub)
        .await
        .unwrap();

    let pool_for_append = pool.clone();
    let resource = ingest.resource;
    let emitter = ingest.emitter;
    let mut append = tokio::spawn(async move {
        let raced = segment(2, "a segment arriving while the scrub holds the row");
        writes::append_block(
            &pool_for_append,
            AppendParams {
                resource,
                block: &raced,
                sources: vec![],
                emitter,
            },
        )
        .await
    });
    let finished_within_window =
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut append).await;
    assert!(
        finished_within_window.is_err(),
        "the append completed while the scrub held R's row — it did not wait"
    );

    scrub.commit().await.unwrap();
    let err = append
        .await
        .expect("the append task must not panic")
        .expect_err("the append must refuse the ingest the scrub ended under it");
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("TF004"),
        "the waiting append re-read the committed state and refused; got {err:#}"
    );
    assert_eq!(ingest_state(&pool, ingest.resource).await, "cancelled");
    assert_eq!(
        live_blocks(&pool, ingest.resource).await,
        2,
        "the raced segment landed nowhere"
    );
}

/// `update_resource`'s whole-body arm treats every state that is not `complete` as an incomplete
/// body: on an ended ingest it bails, naming the state, and writes nothing.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn update_resource_refuses_a_whole_body_write_over_an_ended_ingest(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    for state in TERMINAL {
        let ingest = begin_ingest(&pool, &format!("update-{state}")).await;
        end_ingest(&pool, ingest.resource, state).await;
        let rewrite = "a whole new body";
        let err = writes::update_resource(
            &pool,
            UpdateParams {
                resource: ingest.resource,
                body: Some(rewrite),
                title: None,
                origin_uri: None,
                properties: &[],
                unset_keys: &[],
                chunks: Some(vec![chunk(rewrite, "")]),
                sources: vec![],
                content_block: None,
                rehome_to: None,
                emitter: ingest.emitter,
            },
        )
        .await
        .expect_err("a whole-body write over an ended ingest is refused");
        let message = format!("{err:#}");
        assert!(
            message.contains(&format!(
                "update_resource: resource {} ingest is {state}, not complete",
                ingest.resource
            )),
            "the refusal names the {state} state; got {message}"
        );
        assert_eq!(ingest_state(&pool, ingest.resource).await, state);
        assert_eq!(
            live_blocks(&pool, ingest.resource).await,
            2,
            "the refused update wrote nothing"
        );
    }
}

/// The re-block declines an ended ingest as `IngestEnded` — not `InProgress`, which would tell
/// the caller to retry once the ingest completes, and it never will.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn reblock_declines_an_ended_ingest_as_ingest_ended(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    for state in TERMINAL {
        let ingest = begin_ingest(&pool, &format!("reblock-{state}")).await;
        end_ingest(&pool, ingest.resource, state).await;
        let outcome = writes::reblock_resource(
            &pool,
            ReblockParams {
                resource: ingest.resource,
                emitter: ingest.emitter,
            },
        )
        .await
        .unwrap();
        let ReblockOutcome::Declined { reason } = outcome else {
            panic!("a {state} ingest must decline the re-block; got {outcome:?}");
        };
        assert_eq!(reason.kind, ReblockDeclineKind::IngestEnded);
        assert!(
            reason.detail.contains(&format!("ingest is {state}")),
            "the detail names the {state} state; got {}",
            reason.detail
        );
    }
}
