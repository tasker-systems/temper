#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;

/// GET /api/resources returns a wrapped response with rows, total, and facets.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_list_resources_returns_wrapped_response(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("browse-user-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // Create a resource so there's at least one row.
    app.client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "kb_doc_type_id": common::fixtures::RESEARCH_DOC_TYPE_ID,
            "origin_uri": format!("test://browse-{}", uuid::Uuid::new_v4()),
            "title": "Browse Test Resource",
        }))
        .send()
        .await
        .expect("create failed");

    let resp = app
        .client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list failed");

    assert_eq!(resp.status().as_u16(), 200);

    let body: Value = resp.json().await.expect("expected JSON");

    // Verify wrapped response shape
    assert!(body["rows"].is_array(), "response must have rows array");
    assert!(body["total"].is_number(), "response must have total number");
    assert!(
        body["facets"].is_object(),
        "response must have facets object"
    );
    assert!(
        body["facets"]["doc_type"].is_object(),
        "facets must have doc_type map"
    );

    // Verify rows have the extended fields
    let rows = body["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "should have at least one row");

    let first = &rows[0];
    assert!(
        first["context_name"].is_string(),
        "row must have context_name"
    );
    assert!(
        first["doc_type_name"].is_string(),
        "row must have doc_type_name"
    );
    assert!(
        first["owner_handle"].is_string(),
        "row must have owner_handle"
    );
}

/// GET /api/resources?context_name=temper filters by context name.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_list_resources_filter_by_context_name(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("filter-user-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // Create a resource in the "temper" context.
    app.client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "kb_doc_type_id": common::fixtures::RESEARCH_DOC_TYPE_ID,
            "origin_uri": format!("test://filter-{}", uuid::Uuid::new_v4()),
            "title": "Filter Test Resource",
        }))
        .send()
        .await
        .expect("create failed");

    let resp = app
        .client
        .get(app.url("/api/resources?context_name=temper"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("filtered list failed");

    assert_eq!(resp.status().as_u16(), 200);

    let body: Value = resp.json().await.expect("expected JSON");
    let rows = body["rows"].as_array().unwrap();

    // All returned rows must have context_name == "temper"
    for row in rows {
        assert_eq!(
            row["context_name"].as_str().unwrap(),
            "temper",
            "all rows must be in temper context"
        );
    }
}

/// GET /api/resources?sort=title&order=asc sorts by title ascending.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_list_resources_sort_by_title_asc(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("sort-user-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // Create two resources with known titles.
    for title in ["Alpha Resource", "Zulu Resource"] {
        app.client
            .post(app.url("/api/resources"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&json!({
                "kb_context_id": context_id.to_string(),
                "kb_doc_type_id": common::fixtures::RESEARCH_DOC_TYPE_ID,
                "origin_uri": format!("test://sort-{}", uuid::Uuid::new_v4()),
                "title": title,
            }))
            .send()
            .await
            .expect("create failed");
    }

    let resp = app
        .client
        .get(app.url("/api/resources?sort=title&order=asc"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("sorted list failed");

    assert_eq!(resp.status().as_u16(), 200);

    let body: Value = resp.json().await.expect("expected JSON");
    let rows = body["rows"].as_array().unwrap();

    assert!(rows.len() >= 2, "should have at least 2 rows");

    let titles: Vec<&str> = rows.iter().filter_map(|r| r["title"].as_str()).collect();
    let mut sorted_titles = titles.clone();
    sorted_titles.sort();
    assert_eq!(titles, sorted_titles, "titles must be in ascending order");
}
