//! Body-trio contract tests for PATCH /api/resources/{id}.
//!
//! After Phase 3b's contract tightening, the handler no longer enforces an
//! all-or-nothing guard at the wire level. Instead:
//!
//! - Wire-supplied `content_hash` and `chunks_packed` are intentionally ignored;
//!   the server recomputes them from `content` via `prepare_body_trio`.
//! - Sending `content` without `content_hash` or `chunks_packed` is now valid
//!   (server fills in the pair) — the substrate computes the structural
//!   `body_hash` inline, independent of the embed pipeline.
//! - Sending only `content_hash` or `chunks_packed` without `content` is now a
//!   meta-only no-op — wire hash/chunks fields are silently ignored and the
//!   request succeeds with no body change (200).
//! - A request with all trio fields absent (meta-only update) continues to pass.
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates a test profile and a resource owned by that profile.
/// Returns `(token, resource_id_string)`.
async fn setup_profile_and_resource(app: &common::TestApp) -> (String, String) {
    let email = format!("body-trio-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // Create a resource owned by this profile via POST /api/resources.
    let create_resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://body-trio-{}", uuid::Uuid::new_v4()),
            "title": "Body Trio Test Resource",
            "slug": null
        }))
        .send()
        .await
        .expect("create resource request failed");

    assert_eq!(
        create_resp.status().as_u16(),
        200,
        "resource create must succeed",
    );

    let created: Value = create_resp.json().await.expect("expected JSON from create");
    let resource_id = created["id"]
        .as_str()
        .expect("id field missing")
        .to_string();

    (token, resource_id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// PATCH with content but no content_hash succeeds (200). WS6 collapse retired
/// the all-or-nothing 400 guard: the substrate computes the structural
/// `body_hash` inline (`body_hash_for_body`, Task F), independent of the embed
/// pipeline, so a content PATCH no longer requires a wire-supplied hash/chunks pair.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn patch_with_content_without_pipeline_succeeds(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, resource_id) = setup_profile_and_resource(&app).await;

    let req_body = json!({
        "content": "new body"
        // content_hash and chunks_packed intentionally absent;
        // server ignores any wire-supplied values anyway
    });

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&req_body)
        .send()
        .await
        .expect("PATCH request failed");

    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("expected JSON body");

    assert_eq!(
        status, 200,
        "content PATCH without the pipeline now succeeds (inline body_hash); body: {body}"
    );
    assert!(
        body["body_hash"].is_string(),
        "response must carry the inline-computed body_hash; got: {body}"
    );
}

/// PATCH with only `content_hash` (no `content`) must succeed with a 200:
/// the wire hash is now silently ignored; no body branch fires, so the
/// request is treated as a meta-only no-op.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn patch_accepts_hash_only_as_noop(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, resource_id) = setup_profile_and_resource(&app).await;

    let req_body = json!({
        "content_hash": "sha256:abc"
        // content and chunks_packed intentionally absent
    });

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&req_body)
        .send()
        .await
        .expect("PATCH request failed");

    let status = resp.status().as_u16();

    assert_eq!(
        status,
        200,
        "hash-only (no content) must return 200 — wire hash silently ignored; body: {}",
        resp.text().await.unwrap_or_default()
    );
}

/// PATCH with all three trio fields absent (meta-only update) must be accepted.
///
/// Note: the service layer in Task 2 does not yet process managed_meta — it only
/// updates title/slug. So this test just verifies the handler validation passes
/// through cleanly. The returned status (200) comes from the service returning
/// the unchanged resource row. Future tasks will handle managed_meta application.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn patch_accepts_empty_body_trio(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, resource_id) = setup_profile_and_resource(&app).await;

    let req_body = json!({
        "managed_meta": {
            "stage": "done"
        }
        // no content, content_hash, or chunks_packed
    });

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&req_body)
        .send()
        .await
        .expect("PATCH request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "meta-only update (no trio) must be accepted; body: {}",
        resp.text().await.unwrap_or_default()
    );
}
