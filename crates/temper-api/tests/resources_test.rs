#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;

/// POST /api/resources creates a resource; GET /api/resources lists it.
///
/// Pre-creates the profile and its 'temper' context so that
/// `ingest_service::ingest`'s `contexts_visible_to` gate passes. The JWT
/// `sub` matches the auth link created by `create_test_profile_with_context`
/// (format: `"test|{profile_id}"`), so the middleware resolves the existing
/// profile rather than auto-provisioning a new one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_create_and_list_resources(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email = format!("resource-user-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    // JWT sub must match the auth link installed by create_test_profile_with_context.
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // Create a resource.
    let payload = json!({
        "kb_context_id": context_id.to_string(),
        "doc_type": "research",
        "origin_uri": format!("test://resource-{}", uuid::Uuid::new_v4()),
        "title": "My Integration Test Resource",
        "slug": null,
        "mimetype": "text/markdown"
    });

    let create_resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .send()
        .await
        .expect("create request failed");

    assert_eq!(
        create_resp.status().as_u16(),
        200,
        "create must return 200; body: {}",
        create_resp.text().await.unwrap_or_default()
    );

    let created: Value = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://listed-resource-{}", uuid::Uuid::new_v4()),
            "title": "Listed Resource",
            "slug": null,
            "mimetype": "text/markdown"
        }))
        .send()
        .await
        .expect("second create failed")
        .json()
        .await
        .expect("expected JSON");

    let resource_id = created["id"].as_str().expect("id field missing");

    // List resources — the created resource must appear.
    let list_resp = app
        .client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list request failed");

    assert_eq!(list_resp.status().as_u16(), 200, "list must return 200");

    let list: Value = list_resp.json().await.expect("expected JSON");
    let rows = list["rows"].as_array().expect("expected rows array");
    let ids: Vec<&str> = rows.iter().filter_map(|r| r["id"].as_str()).collect();

    assert!(
        ids.contains(&resource_id),
        "created resource {resource_id} must appear in list; got {ids:?}"
    );
}

/// User A's private resource must NOT be visible to User B.
///
/// Both users have their own 'temper' context so that
/// `ingest_service::ingest`'s `contexts_visible_to` gate passes on create.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_resource_visibility_scoping(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    // User A: pre-create profile + context so ingest can resolve 'temper'.
    let email_a = format!("user-a-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id_a, context_id_a) =
        common::fixtures::create_test_profile_with_context(&pool, &email_a).await;
    let sub_a = format!("test|{profile_id_a}");
    let token_a = common::generate_test_jwt(&sub_a, &email_a);

    let created: Value = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token_a}"))
        .json(&json!({
            "kb_context_id": context_id_a.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://private-{}", uuid::Uuid::new_v4()),
            "title": "User A's Private Resource",
            "slug": null,
            "mimetype": "text/plain"
        }))
        .send()
        .await
        .expect("User A create failed")
        .json()
        .await
        .expect("expected JSON");

    let resource_id = created["id"].as_str().expect("id field missing");

    // User B must not see the resource in their list.
    let sub_b = format!("test-sub-b-{}", uuid::Uuid::new_v4());
    let email_b = format!("user-b-{}@example.com", uuid::Uuid::new_v4());
    let token_b = common::generate_test_jwt(&sub_b, &email_b);

    let list_resp = app
        .client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token_b}"))
        .send()
        .await
        .expect("User B list failed");

    assert_eq!(list_resp.status().as_u16(), 200);

    let list: Value = list_resp.json().await.expect("expected JSON");
    let rows = list["rows"].as_array().expect("expected rows array");
    let ids: Vec<&str> = rows.iter().filter_map(|r| r["id"].as_str()).collect();

    assert!(
        !ids.contains(&resource_id),
        "User A's resource {resource_id} must NOT appear in User B's list; got {ids:?}"
    );
}
