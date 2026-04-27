//! Body-trio validation: content + content_hash + chunks_packed are all-or-nothing.
//!
//! The handler must reject any PATCH /api/resources/{id} where some but not all
//! of the body-trio fields are present, with a 400 and a descriptive message.
//! A request with all trio fields absent (meta-only update) must pass through.
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
    let sub = format!("test-sub-{}", uuid::Uuid::new_v4());
    let email = format!("body-trio-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);

    // Authenticate (this auto-creates the profile).
    let auth_resp = app
        .client
        .get(app.url("/api/auth/me"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("auth/me request failed");
    // 200 or 404 both fine — we just need the profile to exist.
    let _ = auth_resp.status();

    // Create a resource owned by this profile via POST /api/resources.
    let create_resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": common::fixtures::TEMPER_CONTEXT_ID,
            "kb_doc_type_id": common::fixtures::RESEARCH_DOC_TYPE_ID,
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

/// PATCH with content but no content_hash (partial trio) must return 400.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn patch_returns_400_when_content_present_without_hash(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, resource_id) = setup_profile_and_resource(&app).await;

    let req_body = json!({
        "content": "new body",
        "chunks_packed": "blob"
        // content_hash intentionally absent
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

    assert_eq!(status, 400, "partial trio must return 400; body: {body}",);
    let message = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("content_hash"),
        "error message must mention 'content_hash'; got: {message}"
    );
}

/// PATCH with content_hash but no content (another partial trio) must return 400.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn patch_returns_400_when_hash_present_without_content(pool: PgPool) {
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
    let body: Value = resp.json().await.expect("expected JSON body");

    assert_eq!(
        status, 400,
        "hash without content must return 400; body: {body}"
    );
    let message = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("content_hash"),
        "error message must mention 'content_hash'; got: {message}"
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
