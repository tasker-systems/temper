#![cfg(feature = "test-db")]
//! Integration test for `SearchParams.bound_ids` (Task 2 of
//! `.superpowers/sdd/2026-08-12-api-search-resource-bound`).
//!
//! Asserted through the HTTP handler rather than the service layer, because the field's whole
//! purpose is to be reachable by a caller of `/api/search` — mirrors the pattern in
//! `search_context_ref_test.rs` for the other scope narrower, `context_ref`.
//!
//! Seeds three resources in one context that all match the same FTS term, bounds the search to
//! one of them, and asserts the exact arm returns only that resource.

mod common;

use serde_json::json;
use sqlx::PgPool;

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
            "origin_uri": format!("test://search-bound-{}", uuid::Uuid::new_v4()),
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

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_bounded_search_returns_only_the_named_resource(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("search-bound-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // All three share the same distinctive FTS term, so without a bound all three would match.
    let id_a = create_resource_in_context(
        &app,
        &token,
        context_id,
        "ztmpboundword alpha bound-target resource",
    )
    .await;
    let id_b = create_resource_in_context(
        &app,
        &token,
        context_id,
        "ztmpboundword beta unbound resource",
    )
    .await;
    let id_c = create_resource_in_context(
        &app,
        &token,
        context_id,
        "ztmpboundword gamma unbound resource",
    )
    .await;

    let resp = post_search(
        &app,
        &token,
        json!({
            "query": "ztmpboundword",
            "bound_ids": [id_a],
            "limit": 50,
        }),
    )
    .await;

    assert_eq!(
        resp.status().as_u16(),
        200,
        "bounded search must return 200"
    );

    let body: serde_json::Value = resp.json().await.expect("search JSON");
    let returned_ids: Vec<&str> = body["exact"]["hits"]
        .as_array()
        .unwrap_or_else(|| panic!("exact arm must carry hits; got {body}"))
        .iter()
        .filter_map(|r| r["resource"]["id"].as_str())
        .collect();

    assert_eq!(
        returned_ids,
        vec![id_a.as_str()],
        "bound_ids must narrow the search to exactly the named resource; \
         id_b={id_b} id_c={id_c} were seeded but must not appear"
    );
}
