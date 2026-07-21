#![cfg(feature = "test-db")]
//! `PUT /api/cognitive-maps/{id}` end-to-end: drives the REAL Axum server (in-process), real Postgres,
//! real JWT auth. The reconcile request is PRE-EMBEDDED with synthetic (recognizable) chunks, so the
//! handler stays a pure diff+store+event path — NO ONNX. This runs on plain `cargo make test-e2e`
//! (NOT `test-e2e-embed`): the chunk fabrication mirrors `crates/temper-api/tests/reconcile_cogmap_test.rs`.
//!
//! The canonical seed leaves `kb_system_settings.gating_team_slug` NULL → the admin write-gate is a
//! no-op. To exercise BOTH the allow (admin) and deny (non-admin) paths we configure the gating team via
//! `common::enable_invite_only`, which sets `gating_team_slug='temper-system'` AND makes the given
//! profile an owner/admin of that root team. The L0 system-default map is root-team-joined (born by
//! migration `20260625000001`), so the gate applies to it.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::reconcile::{
    CharterDisposition, ReconcileCogmapRequest, ReconcileEdge, ReconcileEntry, ReconcileTelos,
    ReconcileTelosBlock,
};

/// The L0 kernel cognitive map reserved id (birth migration `20260625000001`).
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

// ── chunk fabrication (mirrors crates/temper-api/tests/reconcile_cogmap_test.rs, but standalone: the
//    e2e crate cannot depend on temper-next — it pulls ort/ONNX. We replicate the body-merkle here with
//    sha2, which the substrate's `body_hash_from_chunk_hashes` produces and the create path persists). ──

/// The resource `body_hash` for a caller-supplied chunk set — the create path persists the body as ONE
/// roleless block at seq 0, so the merkle is `sha256_hex(sha256_hex(concat of chunk content_hashes))`.
/// Must byte-match `temper_substrate::content::body_hash_from_chunk_hashes` so the idempotency diff compares
/// like-for-like.
fn body_hash_from_chunk_hashes(chunk_hashes: &[String]) -> String {
    use sha2::{Digest, Sha256};
    fn sha256_hex(s: &str) -> String {
        let mut h = Sha256::new();
        h.update(s.as_bytes());
        format!("{:x}", h.finalize())
    }
    if chunk_hashes.is_empty() {
        return sha256_hex("");
    }
    let concat: String = chunk_hashes.concat();
    sha256_hex(&sha256_hex(&concat))
}

/// Build a synthetic pre-embedded telos block for e2e tests. `role` is the `block_role` tag.
/// `hash_seed` is a short string zero-padded to 64 chars so each block has a stable, distinct
/// `content_hash` — a re-PUT with the same seed is hash-equal (idempotency precondition).
/// Mirrors the pattern in `crates/temper-api/tests/reconcile_charter_test.rs`.
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

/// Build a pre-embedded reconcile entry with a single synthetic chunk. `id` is the STABLE landmark
/// identity (the diff key). `hash_seed` fixes the chunk's content hash (and thus the body merkle), so a
/// re-delivery with the same `id` + `hash_seed` is hash-equal — the idempotency precondition. The
/// chunk's embedding is an inert constant fill — these e2e cases never read it.
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
        embedded_with: None,
    };
    let content_hash = body_hash_from_chunk_hashes(std::slice::from_ref(&chunk.content_hash));
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

/// A two-entry desired-state manifest (cogmap + telos, with a governing edge between them). Ids are
/// fixed (not random) so a re-PUT delivers the SAME ids → matched by id → idempotent.
fn two_entry_request() -> ReconcileCogmapRequest {
    let cogmap_id = Uuid::from_u128(0x019f03f4_2ace_76cb_b1fc_260239dd16a5);
    let telos_id = Uuid::from_u128(0x019f03f4_2acf_7c45_bd12_a2a7152644a1);
    ReconcileCogmapRequest {
        entries: vec![
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
        ],
        fold_resources: vec![],
        fold_edges: vec![],
        telos: None,
    }
}

/// Pre-flight a token by hitting GET /api/profile (auto-provisions the profile), returning its UUID.
async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight request failed");
    assert_eq!(resp.status(), StatusCode::OK, "preflight should succeed");
    let body: serde_json::Value = resp.json().await.expect("preflight json parse");
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door
    // and the ENDPOINT authz (ownership, admin-only, grants) is what the test exercises.
    let __pid: Uuid = body["id"]
        .as_str()
        .expect("profile id missing")
        .parse()
        .expect("profile id parse");
    common::approve(&app.pool, __pid).await;
    __pid
}

// ── (a) admin reconcile is idempotent ───────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_reconcile_l0_is_idempotent(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // The e2e principal is provisioned, then made an owner/admin of the root gating team (and the
    // gating slug is configured so the L0 write-gate is live).
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    let req = two_entry_request();

    // First delivery: empty L0 kernel slice → both landmarks created. Drive the production CLIENT method.
    let out1 = app
        .client
        .cognitive_maps()
        .reconcile_cognitive_map(L0_COGMAP, &req, &Default::default())
        .await
        .expect("admin reconcile should succeed");
    assert_eq!(
        out1.created, 2,
        "both kernel landmarks created on first run"
    );
    assert_eq!((out1.updated, out1.folded), (0, 0));

    // Re-PUT the identical manifest → pure no-op (hashes match): nothing created, both unchanged.
    let out2 = app
        .client
        .cognitive_maps()
        .reconcile_cognitive_map(L0_COGMAP, &req, &Default::default())
        .await
        .expect("second admin reconcile should succeed");
    assert_eq!(
        (out2.created, out2.updated, out2.folded, out2.unchanged),
        (0, 0, 0, 2),
        "a re-delivery of the same request must be a pure no-op"
    );
}

// ── (b) admin reconcile with a telos delivers the charter, re-run is Unchanged (idempotent) ────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_reconcile_delivers_telos_charter_idempotently(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Provision the admin and enable the L0 write-gate (mirrors the sibling test above).
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    // A landmark-free request that delivers only the telos charter (3 blocks).
    let req = ReconcileCogmapRequest {
        entries: vec![],
        fold_resources: vec![],
        fold_edges: vec![],
        telos: Some(three_block_telos()),
    };

    // First delivery: empty L0 telos → charter Created.
    let out1 = app
        .client
        .cognitive_maps()
        .reconcile_cognitive_map(L0_COGMAP, &req, &Default::default())
        .await
        .expect("first admin reconcile with telos should succeed");
    assert_eq!(
        out1.charter,
        CharterDisposition::Created,
        "first delivery onto an empty L0 telos must produce charter == Created"
    );
    // Landmark counts are all zero (no entries in this request).
    assert_eq!(
        (out1.created, out1.updated, out1.folded, out1.unchanged),
        (0, 0, 0, 0),
        "landmark counts must all be zero when entries is empty"
    );

    // Re-PUT the IDENTICAL request: body-merkle matches → charter Unchanged (idempotent through
    // the real Axum + Postgres + JWT stack — the load-bearing assertion of this test).
    let out2 = app
        .client
        .cognitive_maps()
        .reconcile_cognitive_map(L0_COGMAP, &req, &Default::default())
        .await
        .expect("second admin reconcile with telos should succeed");
    assert_eq!(
        out2.charter,
        CharterDisposition::Unchanged,
        "re-delivering the same telos charter through the real HTTP stack must be Unchanged (idempotent)"
    );
}

// ── (c) non-admin reconcile is denied (the handler's own admin gate) ─────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_reconcile_is_denied(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Configure the gating team + make the e2e principal the admin.
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    // A SECOND user with system access (a `watcher` of temper-system) but NOT admin: it passes the
    // system-access middleware and reaches the handler, where `require_cogmap_write_admin` denies it.
    let second_token = common::generate_second_user_jwt();
    let second_id = provision_profile(&app, &second_token).await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role)
         SELECT id, $1, 'watcher' FROM kb_teams WHERE slug = 'temper-system'
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(second_id)
    .execute(&pool)
    .await
    .expect("add second user as watcher");

    let req = two_entry_request();
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/cognitive-maps/{L0_COGMAP}")))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&req)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a non-admin write to the root-team-joined L0 map must be denied by the handler gate"
    );
    let body: serde_json::Value = resp.json().await.expect("json parse");
    assert_eq!(body["error"]["code"], "FORBIDDEN");
}
