#![cfg(feature = "test-db")]
//! Byte identity on reads: `bytes_sha256` / `bytes_len` on `GET /api/resources/{id}/blocks`
//! rows, and `bytes_merkle` on `ResourceView` (show and list rows).
//!
//! The witnesses follow the verification a client is told to run (`ResourceView::bytes_merkle`'s
//! doc): cut the served body at the `/blocks` byte lengths, sha256 each piece, compare it with
//! that block's `bytes_sha256`, then sha256 the concatenated hex and compare it with
//! `bytes_merkle`. Each value is checked against bytes the test computes itself, never against
//! another server value, so a field that drifted from the bytes fails here.
//!
//! The gate: a principal who cannot read the resource gets no row to carry the hash (show 404,
//! list without it, `/blocks` 403), the same answer the content gets.

mod common;

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::hash::sha256_hex;
use temper_core::types::ingest::{
    pack_chunks, AppendBlockPayload, BlocksResponse, FinalizePayload, IngestPayload, PackedChunk,
    SegmentedBegin, SegmentedBeginResponse,
};

/// Section-aligned segments (see `segments_handler_test.rs`): their concatenation chunks back to
/// the two landed chunks, so finalize accepts it.
const SEG1: &str = "# One\n\nfirst segment";
const SEG2: &str = "\n## Two\n\nsecond segment";

fn one_chunk_packed(text: &str) -> String {
    let c = &temper_ingest::chunk::chunk_markdown(text)[0];
    pack_chunks(&[PackedChunk {
        chunk_index: 0,
        header_path: c.header_path.clone(),
        heading_depth: c.heading_depth,
        content: c.content.clone(),
        content_hash: c.content_hash.clone(),
        embedding: vec![0.1_f32; 768],
        embedded_with: None,
    }])
    .expect("pack chunk")
}

async fn auth(pool: &PgPool, tag: &str) -> (String, Uuid) {
    let email = format!("bytes-{tag}-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    (token, context_id)
}

/// A finalized two-block verbatim resource, landed through the segmented HTTP path.
async fn two_block_resource(app: &common::TestApp, pool: &PgPool, token: &str, ctx: Uuid) -> Uuid {
    let begin: SegmentedBeginResponse = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&IngestPayload {
            idempotency_key: None,
            title: "Byte identity".to_string(),
            origin_uri: format!("test://bytes-{}", Uuid::new_v4()),
            context_ref: ctx.to_string(),
            home_cogmap_id: None,
            doc_type_name: "research".to_string(),
            goal: None,
            content_hash: None,
            content: SEG1.to_string(),
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: Some(one_chunk_packed(SEG1)),
            sources: Vec::new(),
            act: Default::default(),
            segmented: Some(SegmentedBegin {
                total_blocks_hint: Some(2),
                block_budget: 262_144,
                source_hash: None,
            }),
        })
        .send()
        .await
        .expect("begin")
        .json()
        .await
        .expect("begin JSON");
    let id = begin.resource_id;
    let append = app
        .client
        .post(app.url(&format!("/api/resources/{id}/blocks")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&AppendBlockPayload {
            seq: 1,
            content: SEG2.to_string(),
            content_hash: sha256_hex(SEG2.as_bytes()),
            chunks_packed: Some(one_chunk_packed(SEG2)),
            sources: Vec::new(),
        })
        .send()
        .await
        .expect("append");
    assert_eq!(append.status().as_u16(), 200, "append lands");
    let body_hash: String = sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    let full = format!("{SEG1}{SEG2}");
    let fin = app
        .client
        .post(app.url(&format!("/api/resources/{id}/finalize")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&FinalizePayload {
            expected_blocks: 2,
            expected_body_hash: body_hash,
            expected_content_hash: Some(sha256_hex(full.as_bytes())),
        })
        .send()
        .await
        .expect("finalize");
    assert_eq!(fin.status().as_u16(), 204, "finalize accepts the bytes");
    id
}

async fn blocks(app: &common::TestApp, token: &str, id: Uuid) -> BlocksResponse {
    app.client
        .get(app.url(&format!("/api/resources/{id}/blocks")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list blocks")
        .json()
        .await
        .expect("blocks JSON")
}

async fn show(app: &common::TestApp, token: &str, id: Uuid, query: &str) -> (u16, Value) {
    let resp = app
        .client
        .get(app.url(&format!("/api/resources/{id}{query}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("show");
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

/// The merkle a client computes from bytes it holds: sha256 of each piece's hex, concatenated.
fn merkle_of(pieces: &[&str]) -> String {
    let hexes: String = pieces.iter().map(|p| sha256_hex(p.as_bytes())).collect();
    sha256_hex(hexes.as_bytes())
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_client_verifies_its_bytes_per_block_and_whole_body(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "verify").await;
    let id = two_block_resource(&app, &pool, &token, ctx).await;

    // Per block: a hash and a length for each live block. The write path re-blocks at headings,
    // so the boundaries are the server's, not the segments'; the body cut below proves them.
    let landed = blocks(&app, &token, id).await;
    assert_eq!(landed.blocks.len(), 2);
    assert!(
        landed
            .blocks
            .iter()
            .all(|b| b.bytes_sha256.is_some() && b.bytes_len.is_some()),
        "every block of a verbatim body carries its byte identity: {landed:?}"
    );
    assert_ne!(
        landed.blocks[0].bytes_sha256.as_deref(),
        Some(landed.blocks[0].content_hash.as_str()),
        "bytes_sha256 is the bytes, not the chunk merkle beside it"
    );

    // Whole body, with the content: cut at the served lengths, every piece and the merkle match.
    let (status, view) = show(&app, &token, id, "?sections=body").await;
    assert_eq!(status, 200);
    let content = view["content"].as_str().expect("content served").to_owned();
    assert_eq!(content, format!("{SEG1}{SEG2}"), "verbatim readback");
    let mut at = 0usize;
    let mut pieces = Vec::new();
    for b in &landed.blocks {
        let n = b.bytes_len.unwrap() as usize;
        let piece = &content[at..at + n];
        assert_eq!(
            Some(sha256_hex(piece.as_bytes())),
            b.bytes_sha256,
            "piece {} verifies",
            b.seq
        );
        pieces.push(piece);
        at += n;
    }
    assert_eq!(at, content.len(), "the lengths tile the body");
    let expected = merkle_of(&pieces);
    assert_eq!(
        view["bytes_merkle"].as_str(),
        Some(expected.as_str()),
        "bytes_merkle beside content is the merkle of that content"
    );

    // Without the body, and on a list row: the same value.
    let (_, bare) = show(&app, &token, id, "").await;
    assert_eq!(bare["bytes_merkle"].as_str(), Some(expected.as_str()));
    let list: Value = app
        .client
        .get(app.url(&format!("/api/resources?context_ref={ctx}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list")
        .json()
        .await
        .expect("list JSON");
    let row = list["rows"]
        .as_array()
        .expect("list rows")
        .iter()
        .find(|r| r["id"].as_str() == Some(id.to_string().as_str()))
        .expect("the resource is listed")
        .clone();
    assert_eq!(row["bytes_merkle"].as_str(), Some(expected.as_str()));

    // The stored bytes move (a heading-only change the chunk merkle does not see): the merkle
    // follows them, and body_hash does not.
    let edited = pieces[0].replacen("# One", "# Uno", 1);
    assert_ne!(edited, pieces[0], "the edit changes block 0's bytes");
    let body_hash_before = view["body_hash"].clone();
    sqlx::query(
        "UPDATE kb_block_content SET content = $2, content_hash = $3 \
          WHERE block_revision_id = (SELECT current_revision_id FROM kb_content_blocks \
                                      WHERE resource_id = $1 AND seq = $4 AND NOT is_folded)",
    )
    .bind(id)
    .bind(&edited)
    .bind(sha256_hex(edited.as_bytes()))
    .bind(landed.blocks[0].seq as i32)
    .execute(&pool)
    .await
    .unwrap();
    let (_, after) = show(&app, &token, id, "?sections=body").await;
    assert_eq!(after["body_hash"], body_hash_before);
    assert_eq!(
        after["bytes_merkle"].as_str(),
        Some(merkle_of(&[edited.as_str(), pieces[1]]).as_str()),
        "the merkle follows the bytes"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_body_without_full_byte_coverage_serves_no_byte_identity(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "derived").await;
    let id = two_block_resource(&app, &pool, &token, ctx).await;
    // Drop the last block's bytes: the body now reads back as a reconstruction (partial coverage).
    sqlx::query(
        "DELETE FROM kb_block_content WHERE block_revision_id = \
           (SELECT current_revision_id FROM kb_content_blocks \
             WHERE resource_id = $1 AND NOT is_folded ORDER BY seq DESC LIMIT 1)",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();

    let landed = blocks(&app, &token, id).await;
    assert!(
        landed.blocks[0].bytes_sha256.is_some(),
        "block 0 keeps its bytes"
    );
    assert_eq!(
        (
            landed.blocks[1].bytes_sha256.clone(),
            landed.blocks[1].bytes_len
        ),
        (None, None),
        "a block with no bytes has no byte identity"
    );
    for query in ["", "?sections=body"] {
        let (status, view) = show(&app, &token, id, query).await;
        assert_eq!(status, 200);
        assert!(
            view.get("bytes_merkle").is_none(),
            "no bytes_merkle over a body that is not wholly verbatim ({query:?}): {view}"
        );
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_hash_is_gated_as_the_bytes_are(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "owner").await;
    let id = two_block_resource(&app, &pool, &token, ctx).await;
    let (stranger, _) = auth(&pool, "stranger").await;

    let (status, view) = show(&app, &stranger, id, "").await;
    assert_eq!(status, 404, "a stranger cannot show it");
    assert!(view.get("bytes_merkle").is_none());
    let blocks_status = app
        .client
        .get(app.url(&format!("/api/resources/{id}/blocks")))
        .header("Authorization", format!("Bearer {stranger}"))
        .send()
        .await
        .expect("list blocks")
        .status()
        .as_u16();
    assert_eq!(blocks_status, 403, "nor list its blocks");
    let list = app
        .client
        .get(app.url(&format!("/api/resources?context_ref={ctx}")))
        .header("Authorization", format!("Bearer {stranger}"))
        .send()
        .await
        .expect("list")
        .text()
        .await
        .unwrap();
    assert!(
        !list.contains(&id.to_string()),
        "nor see it on a list of its context"
    );
}
