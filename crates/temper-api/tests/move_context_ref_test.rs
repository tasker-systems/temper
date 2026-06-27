#![cfg(feature = "test-db")]
//! Integration tests for `PATCH /api/resources/{id}` context-move via ref (Task 6).
//!
//! Verifies:
//! 1. `context_to="@me/<slugB>"` moves the resource to context B (resolves via
//!    `parse_context_ref` + `resolve_context_ref`).
//! 2. A bare name (no `@`/`+` prefix, no UUID form) → 400 Bad Request
//!    (Decision 1: bare names are hard-rejected).

mod common;

use serde_json::json;
use sqlx::PgPool;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Create a resource in the given context, returning its UUID string.
async fn create_resource(
    app: &common::TestApp,
    token: &str,
    context_id: uuid::Uuid,
    title: &str,
) -> String {
    let resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://move-ctx-ref-{}", uuid::Uuid::new_v4()),
            "title": title,
        }))
        .send()
        .await
        .expect("create resource request failed");

    assert!(
        resp.status().is_success(),
        "resource creation must succeed (title={title}), got {}",
        resp.status()
    );

    let body: serde_json::Value = resp.json().await.expect("create response JSON");
    body["id"]
        .as_str()
        .expect("resource id must be a string")
        .to_string()
}

// ─── Test 1: move to @me/<slugB> changes kb_context_id ───────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn move_resource_to_context_by_ref_updates_context(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("move-ref-a-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_a_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // Insert a second context for the same profile with slug 'knowledge'.
    let context_b_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_profiles', $2, 'knowledge', 'knowledge')",
    )
    .bind(context_b_id)
    .bind(profile_id)
    .execute(&app.pool)
    .await
    .expect("insert second context");

    // Create a resource in context A.
    let resource_id = create_resource(&app, &token, context_a_id, "Moveable Resource").await;

    // Verify the resource starts in context A.
    let before: serde_json::Value = app
        .client
        .get(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("GET before failed")
        .json()
        .await
        .expect("GET before JSON");

    assert_eq!(
        before["kb_context_id"].as_str().unwrap_or(""),
        context_a_id.to_string(),
        "resource must start in context A"
    );

    // PATCH with context_to = "@me/knowledge" → should move to context B.
    let patch_resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "context_to": "@me/knowledge" }))
        .send()
        .await
        .expect("PATCH move request failed");

    assert_eq!(
        patch_resp.status().as_u16(),
        200,
        "context-move PATCH must return 200; body: {}",
        patch_resp.text().await.unwrap_or_default()
    );

    // Verify the resource now reports context B's id.
    let after: serde_json::Value = app
        .client
        .get(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("GET after failed")
        .json()
        .await
        .expect("GET after JSON");

    assert_eq!(
        after["kb_context_id"].as_str().unwrap_or(""),
        context_b_id.to_string(),
        "resource must be in context B after move; got: {after}"
    );
}

// ─── Test 2: bare name → 400 Bad Request ─────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn move_with_bare_context_name_returns_bad_request(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("move-bare-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_a_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let resource_id = create_resource(&app, &token, context_a_id, "Bare Name Test").await;

    // PATCH with a bare context name — must be rejected with 400.
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "context_to": "temper" }))
        .send()
        .await
        .expect("PATCH bare-name request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "bare context name must be rejected with 400 Bad Request"
    );
}
