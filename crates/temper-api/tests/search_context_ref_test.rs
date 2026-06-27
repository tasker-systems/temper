#![cfg(feature = "test-db")]
//! Integration tests for the search path context_ref filter (Task 5 — light the
//! dormant `p_context_id` in `unified_search`).
//!
//! Seeds two contexts each with a distinctly-titled resource (both matching the
//! same FTS query). Asserts:
//!
//! 1. `search(query, context_ref="@me/<slugA>")` returns ONLY A's resource
//!    (the pre-fix path passes `context_id: None`, so both would be returned).
//! 2. `search(context_ref="@me/no-such-slug")` → 404 Not Found (unknown slug).
//! 3. `search(context_ref="bare-name")` → 400 Bad Request (closes Beat-2 C1:
//!    unknown bare names must not silently return the full corpus).

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
            "origin_uri": format!("test://search-ctx-ref-{}", uuid::Uuid::new_v4()),
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

/// Run `POST /api/search` with the given params, returning the full response.
async fn post_search(
    app: &common::TestApp,
    token: &str,
    params: serde_json::Value,
) -> reqwest::Response {
    app.client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&params)
        .send()
        .await
        .expect("search request failed")
}

// ─── Test 1: @me/<slugA> with a shared query returns only A's resource ────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn search_by_context_ref_returns_only_that_contexts_resources(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("search-ref-a-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_a_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // Insert a second context owned by the same profile with a distinct slug.
    let context_b_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_profiles', $2, 'search-ctx-b', 'search-ctx-b')",
    )
    .bind(context_b_id)
    .bind(profile_id)
    .execute(&app.pool)
    .await
    .expect("insert context B");

    // Both resources have the same distinctive FTS query term so both would
    // match without a context filter. The filter must isolate A's resource only.
    let id_a = create_resource_in_context(
        &app,
        &token,
        context_a_id,
        "ztmptestword alpha context-a resource",
    )
    .await;
    let id_b = create_resource_in_context(
        &app,
        &token,
        context_b_id,
        "ztmptestword beta context-b resource",
    )
    .await;

    // context_a has slug 'temper' (from the fixture).
    let resp = post_search(
        &app,
        &token,
        json!({
            "query": "ztmptestword",
            "context_ref": "@me/temper",
            "graph_expand": false,
            "limit": 50,
        }),
    )
    .await;

    assert_eq!(
        resp.status().as_u16(),
        200,
        "search with context_ref=@me/temper must return 200"
    );

    let rows: Vec<serde_json::Value> = resp.json().await.expect("search JSON");

    let returned_ids: Vec<&str> = rows
        .iter()
        .filter_map(|r| r["resource_id"].as_str())
        .collect();

    assert!(
        returned_ids.contains(&id_a.as_str()),
        "A's resource must appear in context A search results; got ids: {returned_ids:?}"
    );
    assert!(
        !returned_ids.contains(&id_b.as_str()),
        "B's resource must NOT appear in context A search results; got ids: {returned_ids:?}"
    );
}

// ─── Test 2: @me/no-such-slug → 404 Not Found ────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn search_with_unknown_slug_returns_not_found(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("search-ref-nf-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, _context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let resp = post_search(
        &app,
        &token,
        json!({
            "query": "anything",
            "context_ref": "@me/no-such-slug",
        }),
    )
    .await;

    assert_eq!(
        resp.status().as_u16(),
        404,
        "unknown context slug must return 404 Not Found"
    );
}

// ─── Test 3: bare name → 400 Bad Request (closes Beat-2 C1) ─────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn search_with_bare_context_name_returns_bad_request(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("search-bare-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, _context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // A bare name (no `@` or `+` prefix, no UUID form) must be rejected with
    // 400 and must NOT silently return the full corpus (Beat-2 C1 regression).
    let resp = post_search(
        &app,
        &token,
        json!({
            "query": "anything",
            "context_ref": "temper",
        }),
    )
    .await;

    assert_eq!(
        resp.status().as_u16(),
        400,
        "bare context name must be rejected with 400 Bad Request"
    );
}
