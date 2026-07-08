//! Client-supplied `chunks_packed` is HONORED on create + update; the server embeds
//! server-side only as a FALLBACK when the caller did not supply chunks.
//!
//! This reverses PR#71's "server is the single source of truth for body-trio derivation"
//! contract (which silently discarded the client's `chunks_packed` and always re-embedded).
//!
//! The headline test (`create_honors_client_chunks_no_server_embed`) runs in the plain
//! `test-db` tier — NO ONNX — precisely because honoring client chunks needs no server
//! embedding. The fallback test (`create_without_chunks_falls_back_to_server_embed`) DOES
//! need ONNX at runtime, so it is gated on `test-embed`.
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

/// A synthetic, already-embedded chunk: a KNOWN embedding the server must persist verbatim
/// (a real server re-embed would produce a bge vector, never a constant).
fn synthetic_chunk(index: u32, content: &str, hash_seed: &str, fill: f32) -> PackedChunk {
    // content_hash is stored as opaque text; a 64-hex-ish string keeps it realistic.
    let content_hash = format!("{hash_seed:0>64}");
    PackedChunk {
        chunk_index: index,
        header_path: String::new(),
        heading_depth: 0,
        content: content.to_string(),
        content_hash,
        embedding: vec![fill; 768],
    }
}

/// Fetch the single current chunk's embedding for a resource as a parsed `Vec<f32>`
/// (read back from `kb_chunks.embedding::text`, the pgvector literal `[a,b,...]`).
async fn current_chunk_embedding(pool: &PgPool, resource_id: Uuid) -> Vec<f32> {
    let text: String = sqlx::query_scalar(
        "SELECT embedding::text FROM kb_chunks \
         WHERE resource_id = $1 AND is_current ORDER BY chunk_index LIMIT 1",
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("fetch current chunk embedding");
    text.trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().parse::<f32>().expect("parse embedding component"))
        .collect()
}

/// Build a profile + JWT, return `(token, context_id)`.
async fn auth(pool: &PgPool) -> (String, Uuid) {
    let email = format!("chunks-packed-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    (token, context_id)
}

// ---------------------------------------------------------------------------
// Test #1 — the headline: client chunks honored, NO server embed (test-db tier).
// ---------------------------------------------------------------------------

/// POST /api/ingest with a `chunks_packed` blob carrying a KNOWN embedding must persist that
/// embedding verbatim — proving the server did NOT re-embed. Runs WITHOUT ONNX.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_honors_client_chunks_no_server_embed(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, context_id) = auth(&pool).await;

    // One chunk with a synthetic, recognizable embedding (all 0.5).
    let chunks = vec![synthetic_chunk(0, "Client-chunked body prose.", "aa", 0.5)];
    let payload = IngestPayload {
        title: "Client Chunked".to_string(),
        origin_uri: format!("test://client-chunked-{}", Uuid::new_v4()),
        context_ref: context_id.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        slug: "client-chunked".to_string(),
        content: "Client-chunked body prose.".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&chunks).expect("pack")),
        goal: None,
        act: Default::default(),
        sources: Vec::new(),
    };

    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "ingest-with-chunks must succeed; body: {}",
        resp.text().await.unwrap_or_default()
    );
    let created: Value = resp.json().await.expect("ingest JSON");
    let resource_id = Uuid::parse_str(created["id"].as_str().expect("id missing")).unwrap();

    // The persisted embedding must be the SUPPLIED vector verbatim — not a server re-embed.
    let stored = current_chunk_embedding(&pool, resource_id).await;
    assert_eq!(stored.len(), 768, "embedding must be 768-dim");
    assert!(
        stored.iter().all(|&v| (v - 0.5).abs() < 1e-6),
        "stored embedding must equal the client-supplied vec![0.5; 768] verbatim (server did NOT re-embed); \
         first few: {:?}",
        &stored[..4]
    );
}

// ---------------------------------------------------------------------------
// Test #3 — update honors chunks_packed (test-db tier — no ONNX).
// ---------------------------------------------------------------------------

/// PATCH /api/resources/{id} with `content` + `chunks_packed` must persist the supplied
/// embedding on the revised chunk — the client path needs no server embed.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_honors_client_chunks_no_server_embed(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, context_id) = auth(&pool).await;

    // Create with client chunks (embedding 0.5) — no ONNX.
    let create_chunks = vec![synthetic_chunk(0, "Original client body.", "bb", 0.5)];
    let create_payload = IngestPayload {
        title: "Update Target".to_string(),
        origin_uri: format!("test://update-target-{}", Uuid::new_v4()),
        context_ref: context_id.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        slug: "update-target".to_string(),
        content: "Original client body.".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&create_chunks).expect("pack")),
        goal: None,
        act: Default::default(),
        sources: Vec::new(),
    };
    let created: Value = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&create_payload)
        .send()
        .await
        .expect("create failed")
        .json()
        .await
        .expect("create JSON");
    let resource_id = Uuid::parse_str(created["id"].as_str().expect("id missing")).unwrap();

    // PATCH with new content + new client chunks (embedding 0.25).
    let update_chunks = vec![synthetic_chunk(0, "Revised client body.", "cc", 0.25)];
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "content": "Revised client body.",
            "content_hash": format!("{:0>64}", "cc"),
            "chunks_packed": pack_chunks(&update_chunks).expect("pack"),
        }))
        .send()
        .await
        .expect("PATCH failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "PATCH-with-chunks must succeed; body: {}",
        resp.text().await.unwrap_or_default()
    );

    // The revised chunk must carry the SUPPLIED embedding (0.25), verbatim.
    let stored = current_chunk_embedding(&pool, resource_id).await;
    assert_eq!(stored.len(), 768, "embedding must be 768-dim");
    assert!(
        stored.iter().all(|&v| (v - 0.25).abs() < 1e-6),
        "updated embedding must equal the client-supplied vec![0.25; 768] verbatim (no server re-embed); \
         first few: {:?}",
        &stored[..4]
    );
}

// ---------------------------------------------------------------------------
// Test #2 — the fallback: no chunks supplied ⇒ server embeds (needs ONNX).
// ---------------------------------------------------------------------------

/// POST /api/ingest with content but NO `chunks_packed` must still create a resource with
/// chunks — the server runs the embed pipeline as a fallback. Requires `test-embed` (ONNX).
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_without_chunks_falls_back_to_server_embed(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, context_id) = auth(&pool).await;

    let payload = IngestPayload {
        title: "Server Embedded".to_string(),
        origin_uri: format!("test://server-embedded-{}", Uuid::new_v4()),
        context_ref: context_id.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        slug: "server-embedded".to_string(),
        content: "Fallback prose the server must chunk and embed itself.".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        // No client chunks → the server embeds.
        chunks_packed: None,
        goal: None,
        act: Default::default(),
        sources: Vec::new(),
    };

    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "content-only ingest (server-embed fallback) must succeed; body: {}",
        resp.text().await.unwrap_or_default()
    );
    let created: Value = resp.json().await.expect("ingest JSON");
    let resource_id = Uuid::parse_str(created["id"].as_str().expect("id missing")).unwrap();

    let chunk_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM kb_chunks WHERE resource_id = $1 AND is_current")
            .bind(resource_id)
            .fetch_one(&pool)
            .await
            .expect("count chunks");
    assert!(
        chunk_count > 0,
        "server-embed fallback must persist chunks; got {chunk_count}"
    );
}
