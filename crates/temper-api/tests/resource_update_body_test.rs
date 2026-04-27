//! Body trio path: body_hash + chunk dedupe through PATCH /api/resources/{id}.
//!
//! Tests that when `content`, `content_hash`, and `chunks_packed` are all
//! supplied in a PATCH request, the service:
//!  - persists new chunks and updates `kb_resource_manifests.body_hash`
//!  - short-circuits when content_hash matches stored body_hash (no chunk work)
//!  - handles body trio + managed_meta in a single atomic transaction
//!  - exposes `body_hash` on the returned `ResourceRow`
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::{pack_chunks, PackedChunk};

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

/// Encode a list of chunks into the `chunks_packed` wire format.
fn encode_chunks(chunks: &[PackedChunk]) -> String {
    pack_chunks(chunks).expect("pack_chunks must succeed for test data")
}

/// Create a JWT-authenticated profile + resource with a manifest row containing
/// an initial body. Returns `(token, resource_id_str)`.
///
/// Pattern: generate JWT → call auth/me → create resource via HTTP → seed
/// manifest + chunks directly via sqlx for fixture setup.
async fn setup_resource_with_body(
    app: &common::TestApp,
    pool: &PgPool,
    body_hash: &str,
    chunks: &[PackedChunk],
) -> (String, String) {
    let sub = format!("test-sub-body-{}", Uuid::new_v4());
    let email = format!("body-test-{}@example.com", Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);

    // Auto-provision profile.
    let _ = app
        .client
        .get(app.url("/api/auth/me"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("auth/me failed");

    // Create resource via HTTP.
    let create_resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": common::fixtures::TEMPER_CONTEXT_ID,
            "kb_doc_type_id": common::fixtures::RESEARCH_DOC_TYPE_ID,
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
             (id, profile_id, device_id, kb_context_id, resource_id, event_type, payload, created) \
             VALUES (gen_random_uuid(), \
                 (SELECT owner_profile_id FROM kb_resources WHERE id = $1), \
                 'test-device', $2, $1, 'resource_created', '{}', now()) RETURNING id",
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

/// PATCH with body trio (content + content_hash + chunks_packed) must update
/// `kb_resource_manifests.body_hash` and insert new chunks in kb_chunks.
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

    // PATCH with a new body trio: 3 chunks, different hash.
    let new_hash = "sha256:updated-body-hash-11223344";
    let new_chunks = vec![
        make_packed_chunk(0, "# Updated\n\nNew content here.", "uh1"),
        make_packed_chunk(1, "Second updated section.", "uh2"),
        make_packed_chunk(2, "Third new section.", "uh3"),
    ];
    let chunks_packed = encode_chunks(&new_chunks);

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "content": "# Updated\n\nNew content here.\n\nSecond updated section.\n\nThird new section.",
            "content_hash": new_hash,
            "chunks_packed": chunks_packed,
        }))
        .send()
        .await
        .expect("PATCH request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "body trio PATCH must return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    // Assert: body_hash changed in db.
    let stored_hash_after = fetch_body_hash(&pool, rid).await;
    assert_eq!(
        stored_hash_after, new_hash,
        "body_hash must be updated in kb_resource_manifests"
    );

    // Assert: chunks reflect new state (3 current).
    let new_chunk_count = count_current_chunks(&pool, rid).await;
    assert_eq!(
        new_chunk_count, 3,
        "kb_chunks must reflect new chunks (3 current)"
    );
}

/// PATCH with the SAME content_hash as already stored must short-circuit:
/// no chunk rewire, chunk count unchanged.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_update_with_unchanged_content_short_circuits(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let body_hash = "sha256:same-hash-no-change-aabbcc";
    let chunks = vec![
        make_packed_chunk(0, "Unchanged content.", "h1"),
        make_packed_chunk(1, "Still unchanged.", "h2"),
    ];
    let (token, resource_id) = setup_resource_with_body(&app, &pool, body_hash, &chunks).await;

    let rid = Uuid::parse_str(&resource_id).unwrap();
    let chunk_count_before = count_current_chunks(&pool, rid).await;
    assert_eq!(chunk_count_before, 2, "should have 2 initial chunks");

    // PATCH with the SAME hash — should short-circuit.
    let same_chunks_packed = encode_chunks(&chunks);
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "content": "Unchanged content.\n\nStill unchanged.",
            "content_hash": body_hash,
            "chunks_packed": same_chunks_packed,
        }))
        .send()
        .await
        .expect("PATCH request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "same-hash PATCH must return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    // Chunk count must be unchanged (no rewire happened).
    let chunk_count_after = count_current_chunks(&pool, rid).await;
    assert_eq!(
        chunk_count_after, chunk_count_before,
        "chunk count must not change when content_hash matches stored body_hash"
    );
}

/// PATCH carrying BOTH body trio AND managed_meta must apply both changes in
/// one transaction: body_hash updated AND managed_meta merged.
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

    // PATCH with body trio + managed_meta change.
    let new_hash = "sha256:combined-updated-hash-334455";
    let new_chunks = vec![
        make_packed_chunk(0, "Updated content.", "uh1"),
        make_packed_chunk(1, "New second chunk.", "uh2"),
    ];
    let chunks_packed = encode_chunks(&new_chunks);

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "content": "Updated content.\n\nNew second chunk.",
            "content_hash": new_hash,
            "chunks_packed": chunks_packed,
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

    // Assert: body_hash changed.
    let stored_hash = fetch_body_hash(&pool, rid).await;
    assert_eq!(
        stored_hash, new_hash,
        "body_hash must be updated in combined patch"
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

    // Assert: chunks updated to 2.
    let chunk_count = count_current_chunks(&pool, rid).await;
    assert_eq!(chunk_count, 2, "chunk count must reflect new 2-chunk body");
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
