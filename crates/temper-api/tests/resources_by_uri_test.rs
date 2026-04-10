#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;

/// GET /api/resources/by-uri resolves a resource by UUID ident.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_resolve_by_uri_with_id(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let sub = format!("test-sub-{}", uuid::Uuid::new_v4());
    let email = format!("uri-user-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);

    // First, get the user's auto-provisioned default context.
    let contexts: Vec<Value> = app
        .client
        .get(app.url("/api/contexts"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("contexts list failed")
        .json()
        .await
        .expect("expected JSON");

    // The user should have at least one context (auto-provisioned).
    // Find the user-owned context (not the seed "temper" context).
    let user_context = contexts
        .iter()
        .find(|c| c["kb_owner_table"].as_str() == Some("kb_profiles"))
        .expect("user should have a personal context");
    let context_id = user_context["id"].as_str().unwrap();
    let context_name = user_context["name"].as_str().unwrap();

    // Create a resource in the user's own context.
    let created: Value = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id,
            "kb_doc_type_id": common::fixtures::RESEARCH_DOC_TYPE_ID,
            "origin_uri": format!("test://by-uri-{}", uuid::Uuid::new_v4()),
            "title": "URI Resolution Test",
        }))
        .send()
        .await
        .expect("create failed")
        .json()
        .await
        .expect("expected JSON");

    let resource_id = created["id"].as_str().expect("id field missing");

    // Resolve by URI components with UUID ident and @me owner.
    let url = format!(
        "/api/resources/by-uri?owner=%40me&context={context_name}&doc_type=research&ident={resource_id}"
    );
    let resp = app
        .client
        .get(app.url(&url))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("resolve by-uri failed");

    assert_eq!(resp.status().as_u16(), 200);

    let body: Value = resp.json().await.expect("expected JSON");
    assert_eq!(body["id"].as_str().unwrap(), resource_id);
    assert_eq!(body["title"].as_str().unwrap(), "URI Resolution Test");
}

/// GET /api/resources/by-uri returns 404 for non-existent resource.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_resolve_by_uri_not_found(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let sub = format!("test-sub-{}", uuid::Uuid::new_v4());
    let email = format!("uri-404-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);

    let resp = app
        .client
        .get(app.url(
            "/api/resources/by-uri?owner=%40me&context=temper&doc_type=research&ident=nonexistent-slug",
        ))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("resolve by-uri failed");

    assert_eq!(resp.status().as_u16(), 404);
}
