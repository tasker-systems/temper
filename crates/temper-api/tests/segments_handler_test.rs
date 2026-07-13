#![cfg(feature = "test-db")]
//! HTTP-layer integration tests for the segmented (multi-block) ingest surface (Beat 2 Task 2.3):
//! `POST /api/ingest` (segmented begin) → `POST /api/resources/{id}/blocks` (append) →
//! `GET /api/resources/{id}/blocks` (list) → `POST /api/resources/{id}/finalize`. A cross-profile
//! append must 403 before any write lands.
//!
//! Uses the `TestApp` harness (a live Axum server on a random port backed by a per-test isolated
//! DB) — the same pattern as `relationship_handler_test.rs` / `resource_chunks_packed_test.rs`.
//! Bring-your-own chunks throughout, so these run in the plain `test-db` tier — no ONNX.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::{
    pack_chunks, AppendBlockPayload, BlocksResponse, FinalizePayload, IngestPayload, PackedChunk,
    SegmentedBegin, SegmentedBeginResponse,
};

/// A single pre-chunked, pre-embedded segment (bring-your-own-vectors path) — ONNX-free.
fn one_chunk_packed(text: &str, hash_seed: &str) -> String {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: text.to_owned(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1_f32; 768],
        embedded_with: None,
    };
    pack_chunks(&[chunk]).expect("pack chunk")
}

/// Build a profile + JWT + owned context. Mirrors `resource_chunks_packed_test.rs`'s `auth`.
async fn auth(pool: &PgPool, tag: &str) -> (String, Uuid) {
    let email = format!("segments-{tag}-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    (token, context_id)
}

// ─── Test 1: begin → append → list → finalize, over HTTP ────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn segmented_begin_append_list_finalize_over_http(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, context_id) = auth(&pool, "happy").await;

    // Begin — POST /api/ingest with `segmented` set.
    let begin_payload = IngestPayload {
        title: "Segmented Doc".to_string(),
        origin_uri: format!("test://segmented-{}", Uuid::new_v4()),
        context_ref: context_id.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        goal: None,
        content_hash: None,
        content: "first segment".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(one_chunk_packed("first segment", "aa")),
        sources: Vec::new(),
        act: Default::default(),
        segmented: Some(SegmentedBegin {
            total_blocks_hint: Some(2),
            block_budget: 262_144,
            source_hash: Some("deadbeef".to_string()),
        }),
    };

    let begin_resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&begin_payload)
        .send()
        .await
        .expect("begin request failed");
    assert_eq!(
        begin_resp.status().as_u16(),
        200,
        "segmented begin should return 200"
    );
    let begin: SegmentedBeginResponse = begin_resp.json().await.expect("begin JSON");
    assert_eq!(begin.blocks.len(), 1, "begin reports block 0 only");
    assert_eq!(begin.blocks[0].seq, 0);
    let resource_id = begin.resource_id;

    // Append seq 1.
    let append_payload = AppendBlockPayload {
        seq: 1,
        content: "second segment".to_string(),
        content_hash: temper_core::hash::sha256_hex(b"second segment"),
        chunks_packed: Some(one_chunk_packed("second segment", "bb")),
        sources: Vec::new(),
    };
    let append_resp = app
        .client
        .post(app.url(&format!("/api/resources/{resource_id}/blocks")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&append_payload)
        .send()
        .await
        .expect("append request failed");
    assert_eq!(
        append_resp.status().as_u16(),
        200,
        "append should return 200; body: {}",
        append_resp.text().await.unwrap_or_default()
    );

    // GET /blocks reflects both landed segments.
    let list_resp: BlocksResponse = app
        .client
        .get(app.url(&format!("/api/resources/{resource_id}/blocks")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list request failed")
        .json()
        .await
        .expect("list JSON");
    assert_eq!(list_resp.blocks.len(), 2, "both segments landed");
    assert_eq!(list_resp.blocks[0].seq, 0);
    assert_eq!(list_resp.blocks[1].seq, 1);

    // Finalize against the actual multi-block merkle.
    let actual_hash: String = sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id=$1")
        .bind(resource_id)
        .fetch_one(&pool)
        .await
        .expect("fetch body_hash");
    let finalize_resp = app
        .client
        .post(app.url(&format!("/api/resources/{resource_id}/finalize")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&FinalizePayload {
            expected_blocks: 2,
            expected_body_hash: actual_hash,
        })
        .send()
        .await
        .expect("finalize request failed");
    assert_eq!(
        finalize_resp.status().as_u16(),
        204,
        "finalize should return 204; body: {}",
        finalize_resp.text().await.unwrap_or_default()
    );
}

// ─── Test 2: cross-profile append → 403, no write lands ──────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn append_on_other_profile_resource_returns_403(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    // P begins a segmented ingest.
    let (token_p, context_p) = auth(&pool, "authp").await;
    let begin_payload = IngestPayload {
        title: "P's Segmented Doc".to_string(),
        origin_uri: format!("test://segmented-authp-{}", Uuid::new_v4()),
        context_ref: context_p.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        goal: None,
        content_hash: None,
        content: "first segment".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(one_chunk_packed("first segment", "cc")),
        sources: Vec::new(),
        act: Default::default(),
        segmented: Some(SegmentedBegin {
            total_blocks_hint: None,
            block_budget: 262_144,
            source_hash: None,
        }),
    };
    let begin: SegmentedBeginResponse = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token_p}"))
        .json(&begin_payload)
        .send()
        .await
        .expect("begin request failed")
        .json()
        .await
        .expect("begin JSON");

    // Q gets a token but has no grant on P's resource.
    let (token_q, _context_q) = auth(&pool, "authq").await;
    let append_payload = AppendBlockPayload {
        seq: 1,
        content: "second segment".to_string(),
        content_hash: temper_core::hash::sha256_hex(b"second segment"),
        chunks_packed: Some(one_chunk_packed("second segment", "dd")),
        sources: Vec::new(),
    };
    let resp = app
        .client
        .post(app.url(&format!("/api/resources/{}/blocks", begin.resource_id)))
        .header("Authorization", format!("Bearer {token_q}"))
        .json(&append_payload)
        .send()
        .await
        .expect("append request failed");
    assert_eq!(
        resp.status().as_u16(),
        403,
        "Q appending onto P's resource should return 403; body: {}",
        resp.text().await.unwrap_or_default()
    );

    // No second block landed.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded",
    )
    .bind(begin.resource_id)
    .fetch_one(&pool)
    .await
    .expect("block count");
    assert_eq!(count, 1, "denied append must not land a block");
}

// ─── Test 3: unauthenticated append → 401 ────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn append_without_auth_returns_401(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let resp = app
        .client
        .post(app.url(&format!("/api/resources/{}/blocks", Uuid::new_v4())))
        .json(&AppendBlockPayload {
            seq: 1,
            content: "x".to_string(),
            content_hash: temper_core::hash::sha256_hex(b"x"),
            chunks_packed: Some(one_chunk_packed("x", "ee")),
            sources: Vec::new(),
        })
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "missing auth should return 401"
    );
}
