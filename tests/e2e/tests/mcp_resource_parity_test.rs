#![cfg(feature = "test-db")]

mod common;

use temper_api::services::{context_service, doc_type_service, ingest_service, resource_service};
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
            chunks_packed: None,
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

    assert!(
        matches!(result, Err(temper_api::error::ApiError::NotFound)),
        "expected NotFound, got: {result:?}"
    );
}

/// list_visible filters by doc_type when kb_doc_type_id is set.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_visible_filters_by_doc_type(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    let context = context_service::create(&pool, profile_id, "doctype-filter-test")
        .await
        .expect("context create");

    let research_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("research doc_type");
    let session_id = ingest_service::resolve_doc_type(&pool, "session")
        .await
        .expect("session doc_type");

    let body_hash = content_hash("test content");
    let empty = serde_json::json!({});

    // Create a research resource
    ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "test",
            context_id: context.id,
            doc_type_id: research_id,
            title: "Research Doc",
            slug: Some("research-doc"),
            origin_uri: "test://research",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
            chunks_packed: None,
        },
    )
    .await
    .expect("create research resource");

    // Create a session resource
    ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "test",
            context_id: context.id,
            doc_type_id: session_id,
            title: "Session Doc",
            slug: Some("session-doc"),
            origin_uri: "test://session",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
            chunks_packed: None,
        },
    )
    .await
    .expect("create session resource");

    // List with doc_type filter = research
    let params = resource_service::ResourceListParams {
        kb_context_id: Some(context.id.into()),
        kb_doc_type_id: Some(research_id.into()),
        limit: Some(50),
        ..Default::default()
    };
    let response = resource_service::list_visible(&pool, profile_id.into(), params)
        .await
        .expect("list_visible with doc_type filter");

    assert_eq!(
        response.rows.len(),
        1,
        "should return only the research resource"
    );
    assert_eq!(response.rows[0].title, "Research Doc");
}

/// get_name_by_id resolves a doc type UUID to its name.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn doc_type_get_name_by_id(pool: sqlx::PgPool) {
    let research_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("research doc_type");

    let name = doc_type_service::get_name_by_id(&pool, research_id.into())
        .await
        .expect("get_name_by_id");

    assert_eq!(name, "research");
}
