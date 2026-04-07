#![cfg(feature = "test-db")]

mod common;

use temper_api::services::{context_service, ingest_service, resource_service};
use temper_core::types::ids::ProfileId;

/// Helper: resolve profile ID from e2e test user.
async fn resolve_test_profile(pool: &sqlx::PgPool) -> ProfileId {
    ProfileId::from(
        sqlx::query_scalar!(
            "SELECT id FROM kb_profiles WHERE id IN (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user') LIMIT 1"
        )
        .fetch_one(pool)
        .await
        .expect("profile lookup"),
    )
}

/// Helper: SHA256 hex digest of content.
fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// get_by_slug returns a resource by slug within a context.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn get_by_slug_finds_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    let context = context_service::create(&pool, profile_id, "slug-test")
        .await
        .expect("context create");

    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type");

    let body_hash = content_hash("test content");
    let empty = serde_json::json!({});

    ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "test",
            context_id: context.id,
            doc_type_id,
            title: "Slug Lookup Test",
            slug: Some("slug-lookup-test"),
            origin_uri: "test://slug-lookup",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
        },
    )
    .await
    .expect("create resource");

    let found = resource_service::get_by_slug(
        &pool,
        profile_id.into(),
        "slug-lookup-test",
        context.id.into(),
    )
    .await
    .expect("get_by_slug");

    assert_eq!(found.title, "Slug Lookup Test");
    assert_eq!(found.slug.as_deref(), Some("slug-lookup-test"));
}

/// get_by_slug returns NotFound for non-existent slug.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn get_by_slug_returns_not_found(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;
    let context = context_service::create(&pool, profile_id, "slug-missing-test")
        .await
        .expect("context create");

    let result = resource_service::get_by_slug(
        &pool,
        profile_id.into(),
        "nonexistent-slug",
        context.id.into(),
    )
    .await;

    assert!(result.is_err());
}
