//! Hash-invariant tests — local canonical-form managed_hash must equal
//! server-stored managed_hash for any resource ingested through the API,
//! and stored managed_meta JSONB must always carry `temper-title` and
//! `temper-slug` keys regardless of whether the caller put them there.
//!
//! This is the spec's primary acceptance gate for Phase 5 and the
//! prerequisite for re-enabling show-cache tier-2 in Phase 8.
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use temper_core::hash::compute_managed_hash;
use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use uuid::Uuid;

/// Minimal chunk fixture for ingest. Real embeddings aren't needed — the
/// chunk path is exercised but not asserted on.
fn fake_chunk(content: &str, idx: u32) -> PackedChunk {
    PackedChunk {
        chunk_index: idx,
        header_path: String::new(),
        heading_depth: 0,
        content: content.to_string(),
        content_hash: format!("sha256:fake-{idx}"),
        embedding: vec![0.0_f32; 768],
    }
}

/// Provision a test profile, return its bearer token.
async fn provision_profile(app: &common::TestApp) -> String {
    let sub = format!("hash-invariant-sub-{}", Uuid::new_v4());
    let email = format!("hash-invariant-{}@example.com", Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("profile fetch failed");
    assert_eq!(resp.status().as_u16(), 200, "auto-provision must succeed");
    token
}

/// Ingest a research doc with the given title/slug and `managed_meta`,
/// returning the created resource id.
async fn ingest_research(
    app: &common::TestApp,
    token: &str,
    title: &str,
    slug: &str,
    managed_meta: Option<Value>,
) -> String {
    let chunks = vec![fake_chunk("body content", 0)];
    let chunks_packed = pack_chunks(&chunks).expect("pack_chunks");
    let payload = IngestPayload {
        title: title.to_string(),
        origin_uri: format!("test://hash-invariant-{}", Uuid::new_v4()),
        context_name: "default".to_string(),
        doc_type_name: "research".to_string(),
        slug: slug.to_string(),
        content: "body content".to_string(),
        managed_meta,
        chunks_packed: Some(chunks_packed),
        content_hash: None,
        metadata: None,
        open_meta: None,
    };
    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .send()
        .await
        .expect("ingest request failed");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("ingest response not JSON");
    assert_eq!(status, 200, "ingest must return 200; body: {body}");
    body["id"]
        .as_str()
        .expect("ingest response missing id")
        .to_string()
}

/// Read the stored manifest row for a resource.
async fn read_manifest(pool: &PgPool, resource_id: &str) -> (Value, String) {
    let id = Uuid::parse_str(resource_id).expect("invalid resource id");
    let row = sqlx::query_as::<_, (Value, String)>(
        r#"SELECT managed_meta, managed_hash
             FROM kb_resource_manifests
            WHERE resource_id = $1"#,
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("manifest row fetch failed");
    row
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_stores_temper_title_and_temper_slug_in_managed_meta_jsonb(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let token = provision_profile(&app).await;

    // Caller does NOT put temper-title/temper-slug in managed_meta;
    // the server-side helper must inject them from top-level fields.
    let resource_id = ingest_research(
        &app,
        &token,
        "Hash Invariant Doc",
        "hash-invariant-doc",
        None,
    )
    .await;

    let (managed_meta, _managed_hash) = read_manifest(&pool, &resource_id).await;

    assert_eq!(
        managed_meta.get("temper-title"),
        Some(&Value::String("Hash Invariant Doc".to_string())),
        "stored managed_meta must contain temper-title; got: {managed_meta}"
    );
    assert_eq!(
        managed_meta.get("temper-slug"),
        Some(&Value::String("hash-invariant-doc".to_string())),
        "stored managed_meta must contain temper-slug; got: {managed_meta}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn client_pre_send_canonical_hash_equals_server_post_storage_hash(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let token = provision_profile(&app).await;

    // Client-side: build managed_meta exactly the way the CLI / MCP send-side
    // wiring (Tasks 3 + 4) does — start with user fields, run the helper to
    // inject temper-title / temper-slug, compute the canonical-form hash.
    // This hash is what show-cache tier-2 will compare against the server's
    // stored managed_hash on a future show; the two MUST be byte-identical
    // for tier-2 to short-circuit correctly.
    let title = "Client-Hash Doc";
    let slug = "client-hash-doc";
    let mut canonicalized_managed_meta = json!({"date": "2026-04-10"});
    temper_core::operations::ensure_managed_identity_keys(
        &mut canonicalized_managed_meta,
        title,
        Some(slug),
    );
    let client_pre_send_hash = compute_managed_hash("research", &canonicalized_managed_meta);

    // Send the canonicalized payload through the real /api/ingest path.
    // The server runs strip_system_managed_fields → apply_doc_type_defaults →
    // ensure_managed_identity_keys → validate → store → compute_managed_hash.
    // For caller-canonicalized input with no tier-1 fields, the server's
    // pipeline is byte-identical to the client's compute_managed_hash chain.
    let resource_id = ingest_research(
        &app,
        &token,
        title,
        slug,
        Some(canonicalized_managed_meta.clone()),
    )
    .await;

    let (stored_managed_meta, server_hash) = read_manifest(&pool, &resource_id).await;

    assert_eq!(
        stored_managed_meta, canonicalized_managed_meta,
        "server-stored JSONB must match client-prepared canonical JSONB byte-for-byte"
    );
    assert_eq!(
        server_hash, client_pre_send_hash,
        "client-precomputed canonical hash must equal server-stored hash; \
         this is the precondition for show-cache tier-2 (Phase 8). \
         client={client_pre_send_hash}, server={server_hash}, stored_managed_meta={stored_managed_meta}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn partial_patch_with_top_level_title_change_updates_jsonb_temper_title(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let token = provision_profile(&app).await;

    let resource_id = ingest_research(&app, &token, "Original Title", "original-slug", None).await;

    // Sanity check: the seed put canonical keys in.
    let (initial_managed_meta, _) = read_manifest(&pool, &resource_id).await;
    assert_eq!(
        initial_managed_meta.get("temper-title"),
        Some(&Value::String("Original Title".to_string())),
    );

    // PATCH with ONLY the top-level title changed. The receive-side helper
    // must inject the new title into managed_meta JSONB so columns and JSONB
    // stay in agreement and the stored managed_hash reflects the new state.
    let patch_resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({"title": "Renamed Title"}))
        .send()
        .await
        .expect("patch request failed");
    assert_eq!(
        patch_resp.status().as_u16(),
        200,
        "patch must return 200; body: {}",
        patch_resp.text().await.unwrap_or_default()
    );

    let (after_managed_meta, after_hash) = read_manifest(&pool, &resource_id).await;
    assert_eq!(
        after_managed_meta.get("temper-title"),
        Some(&Value::String("Renamed Title".to_string())),
        "after title-only PATCH, managed_meta JSONB must reflect new title; got: {after_managed_meta}"
    );

    // And the hash invariant still holds.
    let recomputed = compute_managed_hash("research", &after_managed_meta);
    assert_eq!(
        after_hash, recomputed,
        "after PATCH, server hash must still equal local canonical hash"
    );
}
