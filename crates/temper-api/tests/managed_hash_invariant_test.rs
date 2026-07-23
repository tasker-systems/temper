//! Title round-trip tests (ported from the retired Phase-5 managed-meta-JSONB +
//! managed_hash invariant model).
//!
//! WS6 collapse retired BOTH halves of the original premise: (a) the
//! `managed_hash` invariant — `db_backend` sets `managed_hash = None` and
//! GET /meta returns `managed_hash: ""`; and (b) storing the canonical identity
//! keys `temper-title`/`temper-slug` INSIDE managed_meta JSONB — in the substrate
//! those keys have `key_fate == Die` (NOT `Property`), so they are not stored in
//! `kb_properties` and `readback::meta` never surfaces them. The title is now a
//! first-class column (`kb_resources.title`), surfaced on the resource row.
//!
//! What SURVIVES and is still worth pinning is the title round-trip: ingest stores
//! it, a meta-only update preserves it, and a title PATCH updates it. These tests
//! verify that via the resource row (GET /api/resources/{id}).
//!
//! The deleted `client_pre_send_canonical_hash_equals_server_post_storage_hash`
//! test pinned the retired managed_hash byte-equality (the show-cache tier-2
//! precondition) — there is no managed_hash to compare anymore.
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use uuid::Uuid;

/// Minimal chunk fixture for ingest.
fn fake_chunk(content: &str, idx: u32) -> PackedChunk {
    PackedChunk {
        chunk_index: idx,
        header_path: String::new(),
        heading_depth: 0,
        content: content.to_string(),
        content_hash: format!("sha256:fake-{idx}"),
        embedding: vec![0.0_f32; 768],
        embedded_with: None,
    }
}

/// Provision a test profile (auto-provision via GET /api/profile), return its token.
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
    // D11: born Denied. Approve so the gated ingest/update endpoints admit this caller.
    common::fixtures::approve_standing_by_email(&app.pool, &email).await;
    token
}

/// Ingest a research doc with the given title, returning the created id.
async fn ingest_research(app: &common::TestApp, token: &str, title: &str) -> String {
    let chunks = vec![fake_chunk("body content", 0)];
    let chunks_packed = pack_chunks(&chunks).expect("pack_chunks");
    let payload = IngestPayload {
        segmented: None,
        title: title.to_string(),
        origin_uri: format!("test://hash-invariant-{}", Uuid::new_v4()),
        context_ref: "@me/default".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content: "body content".to_string(),
        managed_meta: None,
        chunks_packed: Some(chunks_packed),
        content_hash: None,
        metadata: None,
        open_meta: None,
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
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("ingest response not JSON");
    assert_eq!(status, 200, "ingest must return 200; body: {body}");
    body["id"]
        .as_str()
        .expect("ingest response missing id")
        .to_string()
}

/// Read a resource's row (GET /api/resources/{id}) — the title-bearing projection.
async fn fetch_resource(app: &common::TestApp, token: &str, resource_id: &str) -> Value {
    app.client
        .get(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("show request failed")
        .json()
        .await
        .expect("show JSON")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_stores_title_on_resource(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let token = provision_profile(&app).await;

    let resource_id = ingest_research(&app, &token, "Hash Invariant Doc").await;

    let row = fetch_resource(&app, &token, &resource_id).await;
    assert_eq!(
        row["title"],
        json!("Hash Invariant Doc"),
        "ingest must store the title on the resource row; got: {row}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn meta_only_update_preserves_title(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let token = provision_profile(&app).await;

    let resource_id = ingest_research(&app, &token, "Preserve Original").await;

    // PUT /api/resources/{id}/meta with only a stage change (no title). The title
    // column must survive untouched.
    let put_resp = app
        .client
        .put(app.url(&format!("/api/resources/{resource_id}/meta")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "resource_id": resource_id,
            "managed_meta": {"temper-stage": "done"},
            "open_meta": {},
            "managed_hash": "",
            "open_hash": "",
        }))
        .send()
        .await
        .expect("put meta failed");
    assert_eq!(
        put_resp.status().as_u16(),
        200,
        "PUT meta must return 200; body: {}",
        put_resp.text().await.unwrap_or_default(),
    );

    let row = fetch_resource(&app, &token, &resource_id).await;
    assert_eq!(
        row["title"],
        json!("Preserve Original"),
        "a meta-only update must not change the title; got: {row}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn title_patch_updates_resource_title(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let token = provision_profile(&app).await;

    let resource_id = ingest_research(&app, &token, "Original Title").await;

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

    let row = fetch_resource(&app, &token, &resource_id).await;
    assert_eq!(
        row["title"],
        json!("Renamed Title"),
        "a title PATCH must update the resource title; got: {row}"
    );
}
