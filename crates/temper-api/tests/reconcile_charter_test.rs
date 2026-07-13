#![cfg(feature = "test-db")]
//! The reconcile telos branch: first delivery creates the charter, re-run is unchanged (no events),
//! an edited charter updates, and a landmark-only request leaves the charter Absent.
//!
//! All cases run in the plain `test-db` tier — NO ONNX, NO `test-embed`. Telos blocks are built with
//! synthetic pre-embedded chunks (PackedChunk with constant-fill embedding vectors), mirroring the
//! pattern used in `reconcile_cogmap_test.rs` for landmark entries.
//!
//! (a) first delivery: request with telos (3 blocks) on an empty cogmap telos
//!     → outcome.charter == Created; the telos has 3 role-tagged blocks in order.
//! (b) idempotency: re-run the SAME request → charter == Unchanged AND no new `charter_set` kb_events.
//! (c) update: change one block's content+content_hash → charter == Updated.
//! (d) absent: request with telos: None → charter == Absent; telos still empty; no charter_set events.
//! (e) isolation: a request with BOTH entries (≥1 landmark) and telos → landmark counts correct AND
//!     charter == Created.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::reconcile::{
    CharterDisposition, ReconcileCogmapRequest, ReconcileEntry, ReconcileTelos, ReconcileTelosBlock,
};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, ReconcileCognitiveMap, Surface};

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

// ── builders ────────────────────────────────────────────────────────────────────

/// Build a synthetic pre-embedded telos block. `role` is the block_role tag. `hash_seed` is a short
/// string zero-padded to 64 chars to serve as a stable, distinct `content_hash` per block — exactly
/// as `reconcile_cogmap_test.rs` builds landmark entries. The embedding is an inert 768-f32 fill;
/// the reconcile path stores it verbatim (no ONNX needed here).
fn telos_block(role: &str, content: &str, hash_seed: &str) -> ReconcileTelosBlock {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: content.to_string(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1f32; 768],
        embedded_with: None,
    };
    let chunks_packed = pack_chunks(std::slice::from_ref(&chunk)).expect("pack telos chunk");
    ReconcileTelosBlock {
        role: role.to_string(),
        chunks_packed,
    }
}

/// Build a three-block telos (statement/question/framing) with distinct hash seeds.
fn three_block_telos() -> ReconcileTelos {
    ReconcileTelos {
        blocks: vec![
            telos_block("statement", "What is temper?", "s1"),
            telos_block("question", "How does it work?", "q1"),
            telos_block("framing", "Why does it matter?", "f1"),
        ],
    }
}

fn request_with_telos(telos: Option<ReconcileTelos>) -> ReconcileCogmapRequest {
    ReconcileCogmapRequest {
        entries: vec![],
        fold_resources: vec![],
        fold_edges: vec![],
        telos,
    }
}

fn request_with_entries_and_telos(
    entries: Vec<ReconcileEntry>,
    telos: Option<ReconcileTelos>,
) -> ReconcileCogmapRequest {
    ReconcileCogmapRequest {
        entries,
        fold_resources: vec![],
        fold_edges: vec![],
        telos,
    }
}

async fn system_profile(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle = 'system'")
        .fetch_one(pool)
        .await
        .expect("system profile must exist")
}

async fn backend(pool: &PgPool) -> DbBackend {
    let sys = system_profile(pool).await;
    DbBackend::new(pool.clone(), ProfileId::from(sys))
}

fn cmd(cogmap: Uuid, req: ReconcileCogmapRequest) -> ReconcileCognitiveMap {
    ReconcileCognitiveMap {
        cogmap_id: CogmapId::from(cogmap),
        request: req,
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

/// Count `charter_set` events — the mutation-event measure for charter idempotency.
async fn charter_set_event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'charter_set'",
    )
    .fetch_one(pool)
    .await
    .expect("count charter_set events")
}

/// Resolve the telos resource id for the L0 cogmap.
async fn l0_telos_resource_id(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT telos_resource_id FROM kb_cogmaps WHERE id = $1")
        .bind(L0_COGMAP)
        .fetch_one(pool)
        .await
        .expect("L0 cogmap must have a telos_resource_id")
}

/// Query the live (non-folded) block roles on `resource_id`, ordered by seq. Returns the
/// `block_role` property value for each non-folded block.
async fn live_block_roles(pool: &PgPool, resource_id: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT kp.property_value #>> '{}' \
           FROM kb_content_blocks kcb \
           JOIN kb_properties kp \
             ON kp.owner_table = 'kb_content_blocks' \
            AND kp.owner_id = kcb.id \
            AND kp.property_key = 'block_role' \
            AND NOT kp.is_folded \
          WHERE kcb.resource_id = $1 AND NOT kcb.is_folded \
          ORDER BY kcb.seq",
    )
    .bind(resource_id)
    .fetch_all(pool)
    .await
    .expect("query live block roles")
}

// ── (a) first delivery ────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn first_delivery_creates_charter(pool: PgPool) {
    let be = backend(&pool).await;
    let out = be
        .reconcile_cognitive_map(cmd(
            L0_COGMAP,
            request_with_telos(Some(three_block_telos())),
        ))
        .await
        .expect("first delivery")
        .value;

    assert_eq!(
        out.charter,
        CharterDisposition::Created,
        "first delivery onto an empty telos must be Created"
    );
    // Landmark counts are all zero (no entries in this request).
    assert_eq!(
        (out.created, out.updated, out.unchanged, out.folded),
        (0, 0, 0, 0)
    );

    // The telos resource now carries 3 role-tagged blocks in order: statement, question, framing.
    let telos_id = l0_telos_resource_id(&pool).await;
    let roles = live_block_roles(&pool, telos_id).await;
    assert_eq!(
        roles,
        vec!["statement", "question", "framing"],
        "telos blocks must be role-tagged in delivery order"
    );
}

// ── (b) idempotency: re-run fires zero charter_set events ────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn re_run_is_unchanged_with_no_new_charter_set_events(pool: PgPool) {
    let be = backend(&pool).await;

    // First run: delivers the charter (Created).
    let out1 = be
        .reconcile_cognitive_map(cmd(
            L0_COGMAP,
            request_with_telos(Some(three_block_telos())),
        ))
        .await
        .expect("first delivery")
        .value;
    assert_eq!(out1.charter, CharterDisposition::Created);

    // Snapshot after first run, before second.
    let charter_count_before = charter_set_event_count(&pool).await;
    assert_eq!(
        charter_count_before, 1,
        "one charter_set event after first delivery"
    );

    // Second run: identical request — body-merkle matches → Unchanged.
    let out2 = be
        .reconcile_cognitive_map(cmd(
            L0_COGMAP,
            request_with_telos(Some(three_block_telos())),
        ))
        .await
        .expect("second delivery")
        .value;
    assert_eq!(
        out2.charter,
        CharterDisposition::Unchanged,
        "re-delivering the same charter must be Unchanged"
    );

    let charter_count_after = charter_set_event_count(&pool).await;
    assert_eq!(
        charter_count_after, charter_count_before,
        "re-run must fire ZERO new charter_set events (idempotency); \
         before={charter_count_before} after={charter_count_after}"
    );
}

// ── (c) update: change one block's content → Updated ─────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn updated_block_results_in_updated_disposition(pool: PgPool) {
    let be = backend(&pool).await;

    // Deliver the original three-block charter.
    let out1 = be
        .reconcile_cognitive_map(cmd(
            L0_COGMAP,
            request_with_telos(Some(three_block_telos())),
        ))
        .await
        .expect("first delivery")
        .value;
    assert_eq!(out1.charter, CharterDisposition::Created);

    // Re-deliver with one block changed (different content + hash_seed → different chunk content_hash).
    let updated_telos = ReconcileTelos {
        blocks: vec![
            telos_block("statement", "What is temper? (revised)", "s2"), // changed
            telos_block("question", "How does it work?", "q1"),          // unchanged
            telos_block("framing", "Why does it matter?", "f1"),         // unchanged
        ],
    };
    let out2 = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, request_with_telos(Some(updated_telos))))
        .await
        .expect("update delivery")
        .value;
    assert_eq!(
        out2.charter,
        CharterDisposition::Updated,
        "a changed block must cause charter == Updated"
    );

    // Two charter_set events fired in total (one for each delivery).
    assert_eq!(charter_set_event_count(&pool).await, 2);
}

// ── (d) absent: telos: None leaves charter == Absent ─────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn absent_telos_leaves_charter_absent(pool: PgPool) {
    let be = backend(&pool).await;

    let out = be
        .reconcile_cognitive_map(cmd(
            L0_COGMAP,
            request_with_telos(None), // landmark-only, no telos
        ))
        .await
        .expect("landmark-only request")
        .value;

    assert_eq!(
        out.charter,
        CharterDisposition::Absent,
        "a request with no telos must leave charter == Absent"
    );

    // No charter_set events were fired.
    assert_eq!(
        charter_set_event_count(&pool).await,
        0,
        "a landmark-only request must fire no charter_set events"
    );

    // The telos resource still has no blocks (empty charter).
    let telos_id = l0_telos_resource_id(&pool).await;
    let roles = live_block_roles(&pool, telos_id).await;
    assert!(
        roles.is_empty(),
        "the telos must still have no blocks after a telos-absent request"
    );
}

// ── (e) isolation: entries + telos → landmark counts correct AND charter Created ─────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn entries_and_telos_both_processed_correctly(pool: PgPool) {
    let be = backend(&pool).await;

    // Build a landmark entry (same synthetic-pack approach as reconcile_cogmap_test.rs).
    let landmark_id = Uuid::now_v7();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: "A kernel landmark for isolation test.".to_string(),
        content_hash: format!("{:0>64}", "lm1"),
        embedding: vec![0.1f32; 768],
        embedded_with: None,
    };
    let chunks_packed = pack_chunks(std::slice::from_ref(&chunk)).expect("pack landmark chunk");
    let entry = ReconcileEntry {
        id: landmark_id,
        origin_uri: "temper://kernel/concept/isolation-test".to_string(),
        title: "isolation test landmark".to_string(),
        doc_type: "kernel_landmark".to_string(),
        content_hash: temper_substrate::content::body_hash_from_chunk_hashes(std::slice::from_ref(
            &chunk.content_hash,
        )),
        chunks_packed,
        facets: serde_json::json!({ "layer": "concept" }),
        edges: vec![],
    };

    let out = be
        .reconcile_cognitive_map(cmd(
            L0_COGMAP,
            request_with_entries_and_telos(vec![entry], Some(three_block_telos())),
        ))
        .await
        .expect("combined request")
        .value;

    // Landmark counts: 1 created, rest zero.
    assert_eq!(
        (out.created, out.updated, out.unchanged, out.folded),
        (1, 0, 0, 0),
        "landmark phase must create the entry"
    );
    // Charter phase: Created (first delivery onto empty telos).
    assert_eq!(
        out.charter,
        CharterDisposition::Created,
        "telos phase must create the charter alongside the landmark"
    );

    // Verify the kernel slice saw the landmark.
    let slice = temper_substrate::readback::kernel_slice(&pool, CogmapId::from(L0_COGMAP))
        .await
        .unwrap();
    assert_eq!(slice.len(), 1, "exactly one kernel landmark created");
    assert_eq!(
        slice[0].origin_uri,
        "temper://kernel/concept/isolation-test"
    );

    // Verify the telos has 3 blocks.
    let telos_id = l0_telos_resource_id(&pool).await;
    let roles = live_block_roles(&pool, telos_id).await;
    assert_eq!(roles, vec!["statement", "question", "framing"]);
}
