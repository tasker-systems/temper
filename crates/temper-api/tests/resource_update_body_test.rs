//! Body path: `PATCH /api/resources/{id}` returns and preserves `body_hash`.
//!
//! The body-mutation behaviors — server-side hash recompute, chunk persistence,
//! unchanged-content dedupe/short-circuit, and combined body+managed_meta in one
//! request — are covered end-to-end by the `cloud_update_*` tests in
//! `tests/e2e/tests/cloud_writes_test.rs`. Those drive the real CLI→API path against
//! the current storage model (`kb_resources.body_hash`, chunk tables) and are gated by
//! the "Embed & MCP Round-Trip Tests" CI job. The earlier `test-embed` tests here seeded
//! the retired `kb_resource_manifests` table directly and were removed once that store was
//! dropped (WS6 #166); their coverage lives in the e2e tests above.
//!
//! This file retains only the metadata-only `body_hash`-in-response check, which needs no
//! ONNX (test-db only).
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// PATCH response ResourceRow must include body_hash when a manifest exists.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_response_includes_body_hash(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email = format!("body-hash-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    // Create a resource, then give it a body via a content PATCH — the substrate
    // computes the structural body_hash inline (`body_hash_for_body`, no pipeline).
    let created: Value = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://body-hash-{}", Uuid::new_v4()),
            "title": "Body Hash Test",
            "slug": null
        }))
        .send()
        .await
        .expect("create failed")
        .json()
        .await
        .expect("create JSON");
    let resource_id = created["id"].as_str().expect("id missing");

    let after_content: Value = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "content": "Some content." }))
        .send()
        .await
        .expect("content PATCH failed")
        .json()
        .await
        .expect("content PATCH JSON");
    let stored_hash = after_content["body_hash"].clone();
    assert!(
        stored_hash.is_string(),
        "content PATCH must compute a body_hash; got {after_content}"
    );

    // A managed_meta-only PATCH must still return the (unchanged) body_hash.
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
    assert_eq!(
        body["body_hash"], stored_hash,
        "body_hash must be preserved (and returned) across a managed-only PATCH"
    );
}
