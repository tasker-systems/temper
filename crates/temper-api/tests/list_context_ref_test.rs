#![cfg(feature = "test-db")]
//! Integration tests for the list path context_ref filter (Task 4 — ambiguity fix).
//!
//! Seeds two contexts visible to one principal that share a `name` but have
//! distinct slugs, each with a resource. Asserts:
//!
//! 1. `?context_ref=@me/<slugA>` returns ONLY A's resource (the pre-fix
//!    `c.name = $2` predicate would return both / first-match).
//! 2. `?context_ref=@me/<slugB>` returns ONLY B's resource.
//! 3. `?context_ref=<bare-name>` → 400 Bad Request (Decision 1: bare names rejected).

mod common;

use serde_json::json;
use sqlx::PgPool;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Create a resource in the given context and return the resource id string.
async fn create_resource_in_context(
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
            "origin_uri": format!("test://list-ctx-ref-{}", uuid::Uuid::new_v4()),
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

// ─── Test 1: @me/<slugA> returns only A's resource ───────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_by_context_ref_at_me_slug_returns_only_that_contexts_resources(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("list-ref-a-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_a_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // context_a has slug 'temper' (from the fixture) and name 'temper'.
    // Insert a second context with the SAME name but a different slug.
    let context_b_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_profiles', $2, 'temper-2', 'temper')",
    )
    .bind(context_b_id)
    .bind(profile_id)
    .execute(&app.pool)
    .await
    .expect("insert second same-name context with distinct slug");

    // Create one resource per context with distinct titles.
    let id_a =
        create_resource_in_context(&app, &token, context_a_id, "Resource In Context A").await;
    let id_b =
        create_resource_in_context(&app, &token, context_b_id, "Resource In Context B").await;

    // Filter by context A's ref (@me/temper) — must return only A's resource.
    let resp = app
        .client
        .get(app.url("/api/resources?context_ref=@me/temper"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "list with context_ref=@me/temper must return 200"
    );

    let body: serde_json::Value = resp.json().await.expect("list JSON");
    let rows = body["rows"].as_array().expect("rows must be an array");

    let returned_ids: Vec<&str> = rows
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();

    assert!(
        returned_ids.contains(&id_a.as_str()),
        "A's resource must appear in context A results; got ids: {returned_ids:?}"
    );
    assert!(
        !returned_ids.contains(&id_b.as_str()),
        "B's resource must NOT appear in context A results; got ids: {returned_ids:?}"
    );
}

// ─── Test 2: @me/<slugB> returns only B's resource ───────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_by_context_ref_at_me_slug2_returns_only_that_contexts_resources(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("list-ref-b-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_a_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let context_b_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_profiles', $2, 'temper-2', 'temper')",
    )
    .bind(context_b_id)
    .bind(profile_id)
    .execute(&app.pool)
    .await
    .expect("insert context B");

    let id_a =
        create_resource_in_context(&app, &token, context_a_id, "Resource In Context A").await;
    let id_b =
        create_resource_in_context(&app, &token, context_b_id, "Resource In Context B").await;

    // Filter by context B's ref (@me/temper-2) — must return only B's resource.
    let resp = app
        .client
        .get(app.url("/api/resources?context_ref=@me/temper-2"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "list with context_ref=@me/temper-2 must return 200"
    );

    let body: serde_json::Value = resp.json().await.expect("list JSON");
    let rows = body["rows"].as_array().expect("rows must be an array");

    let returned_ids: Vec<&str> = rows
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();

    assert!(
        returned_ids.contains(&id_b.as_str()),
        "B's resource must appear in context B results; got ids: {returned_ids:?}"
    );
    assert!(
        !returned_ids.contains(&id_a.as_str()),
        "A's resource must NOT appear in context B results; got ids: {returned_ids:?}"
    );
}

// ─── Test 3: bare name → 400 Bad Request ─────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_with_bare_context_name_returns_bad_request(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("list-bare-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, _context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // A bare name (no `@` or `+` prefix, no UUID form) must be rejected.
    let resp = app
        .client
        .get(app.url("/api/resources?context_ref=temper"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "bare context name must be rejected with 400 Bad Request"
    );
}
