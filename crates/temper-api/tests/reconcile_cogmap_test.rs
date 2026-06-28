#![cfg(feature = "test-db")]
//! `DbBackend::reconcile_cognitive_map` is additive-only, provenance-scoped, and idempotent. Drives the
//! backend command DIRECTLY (no HTTP/authz — that is Tasks 5–6), against the L0 reserved cogmap born by
//! migration `20260625000001`. Entries are pre-embedded with synthetic (recognizable) chunks, so every
//! case runs in the plain `test-db` tier — NO ONNX (the reconciler is a pure diff+store+event path).
//!
//! The headline invariant is idempotency (case `re_run_fires_zero_mutation_events`): re-running the same
//! request fires ZERO new MUTATION events. The `admin_reconcile` envelope itself deterministically adds
//! exactly two bookkeeping events per run (`delegated_launch` open + `invocation_closed` close); those
//! are infrastructure, not content drift, so the idempotency measure is the mutation-event count (all
//! event types EXCEPT the two envelope types). The test asserts that measure is unchanged AND that the
//! envelope did fire its two bookkeeping events, so nothing is hidden.

use sqlx::PgPool;
use uuid::Uuid;

use temper_api::backend::DbBackend;
use temper_core::types::ids::ProfileId;
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::reconcile::{
    ReconcileCogmapRequest, ReconcileEdge, ReconcileEntry, ReconcileTombstone,
};
use temper_workflow::operations::{Backend, ReconcileCognitiveMap, Surface};

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

// ── builders ────────────────────────────────────────────────────────────────────

/// Build a pre-embedded reconcile entry. `id` is the STABLE landmark identity (the diff key) — bind it
/// to a variable when an edge must reference this entry as its target. `body`/`hash_seed` define the
/// single synthetic chunk; `content_hash` is computed as the substrate's body merkle over that chunk
/// hash, so a re-delivery with the same `body` produces a hash-equal entry (the idempotency
/// precondition). The chunk's embedding is an inert constant fill — no reconcile test reads it.
fn entry(
    id: Uuid,
    origin_uri: &str,
    title: &str,
    body: &str,
    hash_seed: &str,
    facets: serde_json::Value,
    edges: Vec<ReconcileEdge>,
) -> ReconcileEntry {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.to_string(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1; 768],
    };
    let content_hash = temper_substrate::content::body_hash_from_chunk_hashes(
        std::slice::from_ref(&chunk.content_hash),
    );
    let chunks_packed = pack_chunks(std::slice::from_ref(&chunk)).expect("pack");
    ReconcileEntry {
        id,
        origin_uri: origin_uri.to_string(),
        title: title.to_string(),
        doc_type: "kernel_landmark".to_string(),
        content_hash,
        chunks_packed,
        facets,
        edges,
    }
}

fn request(entries: Vec<ReconcileEntry>) -> ReconcileCogmapRequest {
    ReconcileCogmapRequest {
        entries,
        fold_resources: vec![],
        fold_edges: vec![],
    }
}

async fn system_profile(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle = 'system'")
        .fetch_one(pool)
        .await
        .expect("system profile must exist")
}

/// A backend for the reconcile command. `reconcile_cognitive_map` resolves the system actor itself and
/// ignores `self.profile_id`, but we seed it with the system profile for principled construction.
async fn backend(pool: &PgPool) -> DbBackend {
    let sys = system_profile(pool).await;
    DbBackend::new(pool.clone(), ProfileId::from(sys))
}

fn cmd(cogmap: Uuid, req: ReconcileCogmapRequest) -> ReconcileCognitiveMap {
    ReconcileCognitiveMap {
        cogmap_id: temper_core::types::ids::CogmapId::from(cogmap),
        request: req,
        origin: Surface::ApiHttp,
    }
}

/// Count MUTATION events — every `kb_events` row EXCEPT the `admin_reconcile` envelope's two bookkeeping
/// types (`delegated_launch` open + `invocation_closed` close). This is the idempotency measure.
async fn mutation_event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name NOT IN ('delegated_launch', 'invocation_closed')",
    )
    .fetch_one(pool)
    .await
    .expect("count mutation events")
}

async fn total_event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .expect("count events")
}

async fn is_active(pool: &PgPool, resource_id: Uuid) -> bool {
    sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
        .bind(resource_id)
        .fetch_one(pool)
        .await
        .expect("resource must exist")
}

// ── (a) first delivery ────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn first_delivery_creates_all_entries(pool: PgPool) {
    let be = backend(&pool).await;
    let req = request(vec![
        entry(
            Uuid::now_v7(),
            "temper://kernel/concept/cogmap",
            "cogmap",
            "A cognitive map: a bounded, telos-governed view.",
            "aa",
            serde_json::json!({ "layer": "concept" }),
            vec![],
        ),
        entry(
            Uuid::now_v7(),
            "temper://kernel/concept/telos",
            "telos",
            "A telos: the governing purpose of a map.",
            "bb",
            serde_json::json!({ "layer": "concept" }),
            vec![],
        ),
    ]);

    let out = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, req))
        .await
        .expect("reconcile")
        .value;
    assert_eq!(
        (out.created, out.updated, out.folded, out.unchanged),
        (2, 0, 0, 0)
    );

    let slice = temper_substrate::readback::kernel_slice(&pool, L0_COGMAP.into())
        .await
        .unwrap();
    assert_eq!(slice.len(), 2, "both kernel landmarks now homed to L0");
    // provenance:kernel was stamped (kernel_slice's inner join requires it).
    let uris: Vec<_> = slice.iter().map(|r| r.origin_uri.as_str()).collect();
    assert!(uris.contains(&"temper://kernel/concept/cogmap"));
    assert!(uris.contains(&"temper://kernel/concept/telos"));
}

// ── (b) idempotency: re-run fires zero mutation events ──────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn re_run_fires_zero_mutation_events(pool: PgPool) {
    let be = backend(&pool).await;
    // Bind stable ids so the re-run delivers the SAME ids → matched by id → zero events.
    let cogmap_id = Uuid::now_v7();
    let telos_id = Uuid::now_v7();
    let build = || {
        request(vec![
            entry(
                cogmap_id,
                "temper://kernel/concept/cogmap",
                "cogmap",
                "A cognitive map: a bounded, telos-governed view.",
                "aa",
                serde_json::json!({ "layer": "concept" }),
                vec![ReconcileEdge {
                    to: telos_id,
                    kind: "express".to_string(),
                    polarity: "forward".to_string(),
                    label: Some("governs".to_string()),
                    weight: 1.0,
                }],
            ),
            entry(
                telos_id,
                "temper://kernel/concept/telos",
                "telos",
                "A telos: the governing purpose of a map.",
                "bb",
                serde_json::json!({ "layer": "concept" }),
                vec![],
            ),
        ])
    };

    // First run: creates everything + the edge.
    let out1 = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, build()))
        .await
        .expect("first reconcile")
        .value;
    assert_eq!((out1.created, out1.unchanged), (2, 0));

    // Snapshot AFTER the first run, BEFORE the second.
    let mut_before = mutation_event_count(&pool).await;
    let total_before = total_event_count(&pool).await;

    // Second run: identical request → no creates/updates, all unchanged.
    let out2 = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, build()))
        .await
        .expect("second reconcile")
        .value;
    assert_eq!(
        (out2.created, out2.updated, out2.folded, out2.unchanged),
        (0, 0, 0, 2),
        "a re-delivery of the same request must be a pure no-op"
    );

    let mut_after = mutation_event_count(&pool).await;
    let total_after = total_event_count(&pool).await;

    assert_eq!(
        mut_after, mut_before,
        "the second run must fire ZERO new mutation events (idempotency); \
         before={mut_before} after={mut_after}"
    );
    // The envelope still fired its two bookkeeping events — proven, not hidden.
    assert_eq!(
        total_after - total_before,
        2,
        "the second run fires exactly the envelope's two bookkeeping events (open + close)"
    );
}

// ── (c) update: body change re-blocks, others unchanged ─────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_change_updates_only_that_entry(pool: PgPool) {
    let be = backend(&pool).await;
    // Same stable ids across v1/v2 so the diff matches by id (only the body differs).
    let cogmap_id = Uuid::now_v7();
    let telos_id = Uuid::now_v7();
    let v1 = request(vec![
        entry(
            cogmap_id,
            "temper://kernel/concept/cogmap",
            "cogmap",
            "Original body.",
            "aa",
            serde_json::json!({ "layer": "concept" }),
            vec![],
        ),
        entry(
            telos_id,
            "temper://kernel/concept/telos",
            "telos",
            "A telos: unchanged.",
            "bb",
            serde_json::json!({ "layer": "concept" }),
            vec![],
        ),
    ]);
    be.reconcile_cognitive_map(cmd(L0_COGMAP, v1))
        .await
        .expect("seed");

    // Change ONLY the cogmap entry's body (new content + new chunk hash → new content_hash).
    let v2 = request(vec![
        entry(
            cogmap_id,
            "temper://kernel/concept/cogmap",
            "cogmap",
            "Revised body — substantially different prose.",
            "cc",
            serde_json::json!({ "layer": "concept" }),
            vec![],
        ),
        entry(
            telos_id,
            "temper://kernel/concept/telos",
            "telos",
            "A telos: unchanged.",
            "bb",
            serde_json::json!({ "layer": "concept" }),
            vec![],
        ),
    ]);
    let out = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, v2))
        .await
        .expect("update")
        .value;
    assert_eq!(
        (out.created, out.updated, out.folded, out.unchanged),
        (0, 1, 0, 1),
        "exactly the body-changed entry updates; the other is unchanged"
    );

    // The live body_hash now equals the revised entry's content_hash.
    let expected =
        temper_substrate::content::body_hash_from_chunk_hashes(&[format!("{:0>64}", "cc")]);
    let slice = temper_substrate::readback::kernel_slice(&pool, L0_COGMAP.into())
        .await
        .unwrap();
    let row = slice
        .iter()
        .find(|r| r.origin_uri == "temper://kernel/concept/cogmap")
        .unwrap();
    assert_eq!(row.body_hash.as_deref(), Some(expected.as_str()));
}

// ── (d) provenance isolation: promoted content is untouched + unfoldable-by-absence ─

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promoted_content_is_isolated(pool: PgPool) {
    // Seed a `provenance: promoted` resource homed to the cogmap (NOT part of the kernel slice).
    let (owner, emitter) = temper_substrate::readback::system_actor(&pool)
        .await
        .unwrap();
    let promoted = temper_substrate::writes::create_kernel_resource(
        &pool,
        temper_substrate::writes::KernelCreateParams {
            cogmap: temper_substrate::ids::CogmapId::from(L0_COGMAP),
            resource_id: Uuid::now_v7(),
            title: "promoted landmark",
            origin_uri: "temper://promoted/concept/derived",
            doc_type: "kernel_landmark",
            body: "Content promoted from a working map.",
            chunks: None,
            owner,
            emitter,
        },
    )
    .await
    .unwrap();
    temper_substrate::writes::set_property(
        &pool,
        promoted,
        "provenance",
        &serde_json::json!("promoted"),
        emitter,
    )
    .await
    .unwrap();

    let be = backend(&pool).await;
    let mut_before = mutation_event_count(&pool).await;

    // Reconcile a kernel entry alongside the promoted resource. The promoted one is absent from the
    // request — and O3 says absence NEVER folds, so it must survive untouched.
    let req = request(vec![entry(
        Uuid::now_v7(),
        "temper://kernel/concept/cogmap",
        "cogmap",
        "A cognitive map.",
        "aa",
        serde_json::json!({ "layer": "concept" }),
        vec![],
    )]);
    let out = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, req))
        .await
        .expect("reconcile")
        .value;
    assert_eq!((out.created, out.folded), (1, 0));

    // The promoted resource is still active and still NOT in the kernel slice.
    assert!(is_active(&pool, promoted.uuid()).await);
    let slice = temper_substrate::readback::kernel_slice(&pool, L0_COGMAP.into())
        .await
        .unwrap();
    assert!(
        !slice
            .iter()
            .any(|r| r.origin_uri == "temper://promoted/concept/derived"),
        "promoted content must never surface in the kernel slice"
    );

    // The reconcile fired mutation events ONLY for the kernel create (resource + provenance + facet),
    // never for the promoted resource — it's invisible to the diff.
    let mut_after = mutation_event_count(&pool).await;
    assert!(
        mut_after > mut_before,
        "the kernel create fired events; promoted-touch is excluded by construction"
    );
}

// ── (e) explicit fold: tombstone removes a present kernel resource ──────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn explicit_tombstone_folds_kernel_resource(pool: PgPool) {
    let be = backend(&pool).await;
    let cogmap_id = Uuid::now_v7();
    be.reconcile_cognitive_map(cmd(
        L0_COGMAP,
        request(vec![entry(
            cogmap_id,
            "temper://kernel/concept/cogmap",
            "cogmap",
            "A cognitive map.",
            "aa",
            serde_json::json!({ "layer": "concept" }),
            vec![],
        )]),
    ))
    .await
    .expect("seed");

    // Fold it explicitly (entries empty → absence alone wouldn't fold; the tombstone does).
    let fold_req = ReconcileCogmapRequest {
        entries: vec![],
        fold_resources: vec![ReconcileTombstone { id: cogmap_id }],
        fold_edges: vec![],
    };
    let out = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, fold_req))
        .await
        .expect("fold")
        .value;
    assert_eq!(
        (out.created, out.updated, out.folded, out.unchanged),
        (0, 0, 1, 0)
    );

    let slice = temper_substrate::readback::kernel_slice(&pool, L0_COGMAP.into())
        .await
        .unwrap();
    assert!(
        slice.is_empty(),
        "the folded kernel resource is gone from the slice"
    );
}

// ── (f) FIX #3 fail-fast: an unresolved edge target is rejected with NO writes ───────

/// Count OPEN `admin_reconcile` invocations on L0 — proves the atomic rollback left no stale envelope.
async fn open_admin_reconcile_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_invocations \
          WHERE trigger_kind = 'admin_reconcile' AND status = 'open'",
    )
    .fetch_one(pool)
    .await
    .expect("count open admin_reconcile invocations")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unresolved_edge_target_is_rejected_with_no_writes(pool: PgPool) {
    let be = backend(&pool).await;
    // One entry whose edge points at a target `id` that is NOT any request entry's id and NOT in the
    // (empty) live slice — a genesis-manifest typo. Pre-flight must reject it BadRequest.
    let req = request(vec![entry(
        Uuid::now_v7(),
        "temper://kernel/concept/cogmap",
        "cogmap",
        "A cognitive map.",
        "aa",
        serde_json::json!({ "layer": "concept" }),
        vec![ReconcileEdge {
            to: Uuid::now_v7(),
            kind: "express".to_string(),
            polarity: "forward".to_string(),
            label: None,
            weight: 1.0,
        }],
    )]);

    let err = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, req))
        .await
        .expect_err("an unresolved edge target must be rejected");
    assert!(
        matches!(err, temper_core::error::TemperError::BadRequest(_)),
        "expected BadRequest, got {err:?}"
    );

    // Pre-flight runs before the transaction opens → nothing was written (no resource, no envelope).
    let slice = temper_substrate::readback::kernel_slice(&pool, L0_COGMAP.into())
        .await
        .unwrap();
    assert!(
        slice.is_empty(),
        "a rejected manifest must write NO kernel resources (pre-flight + atomicity)"
    );
    assert_eq!(
        open_admin_reconcile_count(&pool).await,
        0,
        "a rejected manifest must open NO admin_reconcile envelope"
    );
}

// ── (g) atomicity: a mid-transaction failure rolls back EVERYTHING (no partial state) ─

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn failed_reconcile_leaves_no_partial_state(pool: PgPool) {
    let be = backend(&pool).await;
    // This request PASSES pre-flight (the edge target resolves — a self-edge), so the envelope opens and
    // the resource is created in Phase 1; then Phase 2 hits an UNKNOWN edge kind and errors. Because the
    // whole run is one transaction, the create + the envelope-open both roll back.
    let cogmap_id = Uuid::now_v7();
    let req = request(vec![entry(
        cogmap_id,
        "temper://kernel/concept/cogmap",
        "cogmap",
        "A cognitive map.",
        "aa",
        serde_json::json!({ "layer": "concept" }),
        vec![ReconcileEdge {
            to: cogmap_id,                            // resolves (self) → passes pre-flight
            kind: "not_a_real_edge_kind".to_string(), // rejected mid-tx in Phase 2
            polarity: "forward".to_string(),
            label: None,
            weight: 1.0,
        }],
    )]);

    let err = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, req))
        .await
        .expect_err("an unknown edge kind must fail the reconcile");
    assert!(
        matches!(err, temper_core::error::TemperError::BadRequest(_)),
        "expected BadRequest (unknown edge kind), got {err:?}"
    );

    // Atomic rollback: the Phase-1 create is gone AND no admin_reconcile envelope remains open.
    let slice = temper_substrate::readback::kernel_slice(&pool, L0_COGMAP.into())
        .await
        .unwrap();
    assert!(
        slice.is_empty(),
        "the Phase-1 create must have rolled back with the failed transaction"
    );
    assert_eq!(
        open_admin_reconcile_count(&pool).await,
        0,
        "the opened envelope must have rolled back — no stale open invocation"
    );
}

/// Regression guard for the CLI/server hash-source split. The wire `content_hash` is ADVISORY: the
/// server diffs on the chunk-merkle it computes from `chunks_packed` (the same `body_hash_from_chunk_hashes`
/// the substrate stores), NEVER on `content_hash`. The operator CLI fills `content_hash` via
/// `compute_body_hash` (a whole-body `sha256:`-prefixed hash) which can never equal the stored
/// chunk-merkle — so if the diff trusted it, every release would re-block every landmark. Here we
/// deliver an entry whose `content_hash` is exactly that CLI-style value and assert the re-run is
/// UNCHANGED (before the fix this reported `updated=1`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn wire_content_hash_is_advisory_rerun_is_unchanged(pool: PgPool) {
    let be = backend(&pool).await;
    let mut e = entry(
        Uuid::now_v7(),
        "temper://kernel/concept/cogmap",
        "cogmap",
        "A cognitive map: a bounded, telos-governed view.",
        "aa",
        serde_json::json!({ "layer": "concept" }),
        vec![],
    );
    // Override with the value the REAL operator CLI emits (whole-body hash, `sha256:`-prefixed) — which
    // never equals the stored chunk-merkle. A diff that trusted this would re-block on every run.
    e.content_hash =
        temper_core::hash::compute_body_hash("A cognitive map: a bounded, telos-governed view.");

    let out1 = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, request(vec![e.clone()])))
        .await
        .expect("first delivery")
        .value;
    assert_eq!(out1.created, 1, "first delivery creates the entry");

    let out2 = be
        .reconcile_cognitive_map(cmd(L0_COGMAP, request(vec![e])))
        .await
        .expect("re-delivery")
        .value;
    assert_eq!(
        (out2.created, out2.updated, out2.unchanged),
        (0, 0, 1),
        "wire content_hash is advisory — the server diffs on the chunk-merkle, so a same-body re-run \
         must be UNCHANGED even when content_hash is the CLI's (non-matching) whole-body hash",
    );
}
