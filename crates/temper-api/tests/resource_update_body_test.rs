//! Body path: body_hash + chunk dedupe through PATCH /api/resources/{id}.
//!
//! After Phase 3b's contract tightening, clients send only `content`; the
//! server recomputes `content_hash` and `chunks_packed` via `prepare_body_trio`.
//! Wire-supplied `content_hash`/`chunks_packed` fields are silently ignored.
//!
//! Tests that verify body persistence (hash update, chunk dedupe, combined
//! body+meta) are gated on `test-embed` because `prepare_body_trio` requires
//! the `ingest-pipeline` feature (ONNX Runtime).
//!
//! The `update_response_includes_body_hash` test is safe under `test-db` only
//! because it does a meta-only PATCH (no content body sent).
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::PackedChunk;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal `PackedChunk` suitable for test payloads.
/// Uses a 768-dim zero embedding (matches the db schema `vector(768)`).
fn make_packed_chunk(index: u32, content: &str, content_hash: &str) -> PackedChunk {
    PackedChunk {
        chunk_index: index,
        header_path: String::new(),
        heading_depth: 0,
        content: content.to_string(),
        content_hash: content_hash.to_string(),
        embedding: vec![0.0_f32; 768],
    }
}

/// Create a JWT-authenticated profile + resource with a manifest row containing
/// an initial body. Returns `(token, resource_id_str)`.
///
/// Pattern: create profile with context → generate matching JWT → create
/// resource via HTTP → seed manifest + chunks directly via sqlx for fixture setup.
async fn setup_resource_with_body(
    app: &common::TestApp,
    pool: &PgPool,
    body_hash: &str,
    chunks: &[PackedChunk],
) -> (String, String) {
    let email = format!("body-test-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // Create resource via HTTP.
    let create_resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://body-trio-{}", Uuid::new_v4()),
            "title": "Body Trio Test",
            "slug": null
        }))
        .send()
        .await
        .expect("create resource failed");

    assert_eq!(
        create_resp.status().as_u16(),
        200,
        "resource create must succeed"
    );

    let created: Value = create_resp.json().await.expect("expected JSON");
    let resource_id_str = created["id"]
        .as_str()
        .expect("id field missing")
        .to_string();
    let resource_id = Uuid::parse_str(&resource_id_str).expect("invalid uuid");

    // Seed the manifest row with the given body_hash.
    let managed_meta = json!({});
    let open_meta = json!({});
    let managed_hash = temper_core::hash::compute_managed_hash("research", &managed_meta);
    let open_hash = temper_core::hash::compute_open_hash(&open_meta);
    sqlx::query(
        r#"INSERT INTO kb_resource_manifests
            (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
           VALUES ($1, $2, $3, $4, $5, $6, now())
           ON CONFLICT (resource_id) DO UPDATE
               SET body_hash = $2, updated = now()"#,
    )
    .bind(resource_id)
    .bind(body_hash)
    .bind(&managed_meta)
    .bind(&open_meta)
    .bind(&managed_hash)
    .bind(&open_hash)
    .execute(pool)
    .await
    .expect("seed manifest row");

    // Seed initial chunks using the persist_resource_chunks SQL function.
    // We need a KB event + audit row first (required FK on kb_resource_chunks).
    if !chunks.is_empty() {
        let context_id: Uuid =
            sqlx::query_scalar("SELECT kb_context_id FROM kb_resources WHERE id = $1")
                .bind(resource_id)
                .fetch_one(pool)
                .await
                .expect("fetch context_id");

        let event_id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_events \
             (id, profile_id, device_id, kb_context_id, resource_id, event_type_id, payload, created) \
             VALUES (gen_random_uuid(), \
                 (SELECT owner_profile_id FROM kb_resources WHERE id = $1), \
                 'test-device', $2, $1, (SELECT id FROM kb_event_types WHERE name = 'resource_created'), '{}', now()) RETURNING id",
        )
        .bind(resource_id)
        .bind(context_id)
        .fetch_one(pool)
        .await
        .expect("insert seed event");

        let profile_id: Uuid =
            sqlx::query_scalar("SELECT owner_profile_id FROM kb_resources WHERE id = $1")
                .bind(resource_id)
                .fetch_one(pool)
                .await
                .expect("fetch profile_id");

        let audit_id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resource_audits \
             (resource_id, event_id, profile_id, device_id, body_hash, managed_hash, open_hash, action) \
             VALUES ($1, $2, $3, 'test-device', $4, 'mh', 'oh', 'create') RETURNING id",
        )
        .bind(resource_id)
        .bind(event_id)
        .bind(profile_id)
        .bind(body_hash)
        .fetch_one(pool)
        .await
        .expect("insert seed audit");

        let chunks_json = temper_core::types::ingest::chunks_to_jsonb(chunks);
        let _: Uuid = sqlx::query_scalar(
            "SELECT persist_resource_chunks($1::uuid, $2::uuid, $3::text, $4::jsonb)",
        )
        .bind(resource_id)
        .bind(audit_id)
        .bind(body_hash)
        .bind(&chunks_json)
        .fetch_one(pool)
        .await
        .expect("seed initial chunks");
    }

    (token, resource_id_str)
}

/// Count current (non-superseded) chunks for a resource in kb_chunks.
#[cfg(feature = "test-embed")]
async fn count_current_chunks(pool: &PgPool, resource_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true",
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("count current chunks")
}

/// Fetch body_hash from kb_resource_manifests.
#[cfg(feature = "test-embed")]
async fn fetch_body_hash(pool: &PgPool, resource_id: Uuid) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("fetch body_hash")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// PATCH with `content` (no wire hash/chunks) must update
/// `kb_resource_manifests.body_hash` and insert new chunks in kb_chunks.
/// The server recomputes the hash and chunks from content via `prepare_body_trio`.
/// Requires `test-embed` (ONNX Runtime).
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_update_changes_body_hash_and_persists_chunks(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    // Seed with initial body hash + 2 chunks.
    let initial_hash = "sha256:initial-body-hash-aabbccdd";
    let initial_chunks = vec![
        make_packed_chunk(0, "# Original\n\nContent here.", "h1"),
        make_packed_chunk(1, "Second section content.", "h2"),
    ];
    let (token, resource_id) =
        setup_resource_with_body(&app, &pool, initial_hash, &initial_chunks).await;

    let rid = Uuid::parse_str(&resource_id).unwrap();

    // Verify initial state.
    let initial_chunk_count = count_current_chunks(&pool, rid).await;
    assert_eq!(initial_chunk_count, 2, "should have 2 initial chunks");
    let stored_hash_before = fetch_body_hash(&pool, rid).await;
    assert_eq!(stored_hash_before, initial_hash);

    // PATCH with content only — server recomputes hash + chunks.
    // Wire content_hash/chunks_packed are intentionally omitted (they'd be ignored anyway).
    let new_content =
        "# Updated\n\nNew content here.\n\nSecond updated section.\n\nThird new section.";
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "content": new_content,
        }))
        .send()
        .await
        .expect("PATCH request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "content-only PATCH must return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    // Assert: body_hash changed (server recomputed a real sha256).
    let stored_hash_after = fetch_body_hash(&pool, rid).await;
    assert_ne!(
        stored_hash_after, initial_hash,
        "body_hash must change after content update"
    );
    assert!(
        stored_hash_after.starts_with("sha256:"),
        "server-computed hash must be sha256-prefixed; got: {stored_hash_after}"
    );

    // Assert: chunks reflect new state (pipeline produces chunks from content).
    let new_chunk_count = count_current_chunks(&pool, rid).await;
    assert!(
        new_chunk_count > 0,
        "kb_chunks must be populated after body update; count: {new_chunk_count}"
    );
}

/// PATCH with the SAME content sent twice must short-circuit:
/// no chunk rewire, chunk count unchanged. The server computes the same hash
/// from the same content and skips the chunk update.
/// Requires `test-embed` (ONNX Runtime).
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_update_with_unchanged_content_short_circuits(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    // We can't pre-seed with a meaningful hash here because the pipeline will
    // compute its own. Instead: do a first PATCH to establish content, then
    // PATCH again with the same content and assert chunk count is unchanged.
    let empty_chunks: Vec<PackedChunk> = vec![];
    let (token, resource_id) =
        setup_resource_with_body(&app, &pool, "sha256:placeholder", &empty_chunks).await;

    let rid = Uuid::parse_str(&resource_id).unwrap();

    let content = "Unchanged content.\n\nStill unchanged.";

    // First PATCH: establish content.
    let resp1 = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "content": content }))
        .send()
        .await
        .expect("first PATCH failed");
    assert_eq!(
        resp1.status().as_u16(),
        200,
        "first content PATCH must succeed; body: {}",
        resp1.text().await.unwrap_or_default()
    );

    let chunk_count_after_first = count_current_chunks(&pool, rid).await;

    // Second PATCH: same content → server computes same hash → short-circuit.
    let resp2 = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "content": content }))
        .send()
        .await
        .expect("second PATCH failed");

    assert_eq!(
        resp2.status().as_u16(),
        200,
        "same-content PATCH must return 200; body: {}",
        resp2.text().await.unwrap_or_default()
    );

    // Chunk count must be unchanged (no rewire happened).
    let chunk_count_after_second = count_current_chunks(&pool, rid).await;
    assert_eq!(
        chunk_count_after_second, chunk_count_after_first,
        "chunk count must not change when content is identical (hash short-circuit)"
    );
}

/// PATCH carrying BOTH content AND managed_meta must apply both changes in
/// one transaction: body_hash updated (server-computed) AND managed_meta merged.
/// Requires `test-embed` (ONNX Runtime).
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_update_combined_with_managed_meta_in_one_tx(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    // Seed with initial body.
    let initial_hash = "sha256:combined-initial-hash-001122";
    let initial_chunks = vec![make_packed_chunk(0, "Initial content.", "ih1")];
    let (token, resource_id) =
        setup_resource_with_body(&app, &pool, initial_hash, &initial_chunks).await;

    let rid = Uuid::parse_str(&resource_id).unwrap();

    // Seed managed_meta with a stage.
    let managed_meta = json!({ "temper-stage": "in-progress" });
    let managed_hash = temper_core::hash::compute_managed_hash("research", &managed_meta);
    let open_meta = json!({});
    let open_hash = temper_core::hash::compute_open_hash(&open_meta);
    sqlx::query(
        r#"UPDATE kb_resource_manifests
           SET managed_meta = $1, managed_hash = $2, open_meta = $3, open_hash = $4
           WHERE resource_id = $5"#,
    )
    .bind(&managed_meta)
    .bind(&managed_hash)
    .bind(&open_meta)
    .bind(&open_hash)
    .bind(rid)
    .execute(&pool)
    .await
    .expect("update managed_meta fixture");

    // PATCH with content + managed_meta change. Wire hash/chunks intentionally
    // omitted — server recomputes them from content.
    let new_content = "Updated content.\n\nNew second chunk.";
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "content": new_content,
            "managed_meta": { "temper-stage": "done" },
        }))
        .send()
        .await
        .expect("PATCH request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "combined PATCH must return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    // Assert: body_hash changed (server recomputed from new content).
    let stored_hash = fetch_body_hash(&pool, rid).await;
    assert_ne!(
        stored_hash, initial_hash,
        "body_hash must change after content update"
    );
    assert!(
        stored_hash.starts_with("sha256:"),
        "server-computed hash must be sha256-prefixed; got: {stored_hash}"
    );

    // Assert: managed_meta stage changed to "done".
    let stored_meta: Value = sqlx::query_scalar::<_, Value>(
        "SELECT managed_meta FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(rid)
    .fetch_one(&pool)
    .await
    .expect("fetch managed_meta");
    assert_eq!(
        stored_meta["temper-stage"],
        json!("done"),
        "managed_meta stage must be merged in the same transaction"
    );

    // Assert: chunks populated from new content.
    let chunk_count = count_current_chunks(&pool, rid).await;
    assert!(
        chunk_count > 0,
        "kb_chunks must be populated after combined body+meta update; count: {chunk_count}"
    );
}

/// PATCH response ResourceRow must include body_hash when a manifest exists.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_response_includes_body_hash(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let body_hash = "sha256:response-hash-test-aabbcc1122";
    let initial_chunks = vec![make_packed_chunk(0, "Some content.", "c1")];
    let (token, resource_id) =
        setup_resource_with_body(&app, &pool, body_hash, &initial_chunks).await;

    // PATCH with managed_meta only — body_hash should still be returned.
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "managed_meta": { "temper-stage": "done" }
        }))
        .send()
        .await
        .expect("PATCH request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "managed_meta-only PATCH must return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    let body: Value = resp.json().await.expect("expected JSON response");
    assert!(
        body.get("body_hash").is_some(),
        "ResourceRow response must include body_hash field"
    );
    assert!(
        !body["body_hash"].is_null(),
        "body_hash must be populated (not null) when manifest exists"
    );
    assert_eq!(
        body["body_hash"],
        json!(body_hash),
        "body_hash in response must match stored manifest value"
    );
}
