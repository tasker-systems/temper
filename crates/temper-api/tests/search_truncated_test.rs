#![cfg(feature = "test-db")]
//! Integration test for each search arm's `truncated`: the arm's own word that hits follow this
//! page, read from the probe row past it.
//!
//! The case that matters is the page that ends exactly on the last match. A caller inferring
//! "more" from `hits.len() == limit` reads that page as truncated; the arm must not.

mod common;

use serde_json::json;
use sqlx::PgPool;

async fn create_resource_in_context(
    app: &common::TestApp,
    token: &str,
    context_id: uuid::Uuid,
    title: &str,
) {
    let resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://search-truncated-{}", uuid::Uuid::new_v4()),
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
}

/// The exact arm's hit count and `truncated` for one page of the shared term.
async fn exact_page(app: &common::TestApp, token: &str, limit: i64, offset: i64) -> (usize, bool) {
    let resp = app
        .client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "query": "ztmptruncword", "limit": limit, "offset": offset }))
        .send()
        .await
        .expect("search request failed");
    assert_eq!(resp.status().as_u16(), 200, "search must return 200");
    let body: serde_json::Value = resp.json().await.expect("search JSON");
    let hits = body["exact"]["hits"]
        .as_array()
        .unwrap_or_else(|| panic!("exact arm must carry hits; got {body}"))
        .len();
    let truncated = body["exact"]["truncated"]
        .as_bool()
        .unwrap_or_else(|| panic!("exact arm must carry truncated; got {body}"));
    assert!(
        body["wide"]["truncated"].is_boolean(),
        "wide arm must carry truncated; got {body}"
    );
    (hits, truncated)
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn each_page_says_whether_more_hits_follow_it(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("search-truncated-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    for name in ["alpha", "beta", "gamma"] {
        create_resource_in_context(&app, &token, context_id, &format!("ztmptruncword {name}"))
            .await;
    }

    assert_eq!(
        exact_page(&app, &token, 2, 0).await,
        (2, true),
        "two of three: more follow"
    );
    assert_eq!(
        exact_page(&app, &token, 3, 0).await,
        (3, false),
        "a page ending exactly on the last match is not truncated"
    );
    assert_eq!(
        exact_page(&app, &token, 2, 2).await,
        (1, false),
        "the last page has nothing after it"
    );
}
