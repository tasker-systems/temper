#![cfg(feature = "test-db")]

mod common;

use temper_api::services::{context_service, ingest_service};
use temper_core::types::ids::ProfileId;

/// Helper: SHA256 hex digest of content.
fn sha2_hex(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// create_resource_with_manifest creates resource + manifest + event.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_resource_with_manifest_inserts_all_records(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = ProfileId::from(
        sqlx::query_scalar!(
            "SELECT id FROM kb_profiles WHERE id IN (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user') LIMIT 1"
        )
        .fetch_one(&pool)
        .await
        .expect("profile lookup"),
    );

    // Create the context first so resolve_by_name can find it.
    context_service::create(&pool, profile_id, "mcp-test")
        .await
        .expect("context create");

    let context = context_service::resolve_by_name(&pool, profile_id, "mcp-test")
        .await
        .expect("context");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type");

    let content = "# MCP Test\n\nThis is test content from MCP ingest.";
    let body_hash = format!("sha256:{}", sha2_hex(content));
    let empty = serde_json::json!({});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        profile_id,
        "mcp-test",
        context.id,
        doc_type_id,
        "MCP Test Resource",
        Some("mcp-test-resource"),
        "mcp://test/create",
        &body_hash,
        &empty,
        &empty,
    )
    .await
    .expect("create_resource_with_manifest");

    assert_eq!(resource.title, "MCP Test Resource");
    assert!(resource.is_active);

    // Verify manifest
    let manifest_hash: String = sqlx::query_scalar!(
        "SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1",
        *resource.id,
    )
    .fetch_one(&pool)
    .await
    .expect("manifest lookup");
    assert_eq!(manifest_hash, body_hash);

    // Verify event
    let event_count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_events WHERE resource_id = $1 AND event_type = 'resource_created'",
        *resource.id,
    )
    .fetch_one(&pool)
    .await
    .expect("event count")
    .unwrap_or(0);
    assert_eq!(event_count, 1);
}

/// Dedup: same body hash returns existing resource.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn find_by_body_hash_returns_existing(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = ProfileId::from(
        sqlx::query_scalar!(
            "SELECT id FROM kb_profiles WHERE id IN (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user') LIMIT 1"
        )
        .fetch_one(&pool)
        .await
        .expect("profile lookup"),
    );

    // Create the context first so resolve_by_name can find it.
    context_service::create(&pool, profile_id, "dedup-test")
        .await
        .expect("context create");

    let context = context_service::resolve_by_name(&pool, profile_id, "dedup-test")
        .await
        .expect("context");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type");

    let content = "# Dedup Test\n\nIdentical content for dedup testing.";
    let body_hash = format!("sha256:{}", sha2_hex(content));
    let empty = serde_json::json!({});

    let first = ingest_service::create_resource_with_manifest(
        &pool,
        profile_id,
        "test",
        context.id,
        doc_type_id,
        "First",
        None,
        "mcp://test/dedup-1",
        &body_hash,
        &empty,
        &empty,
    )
    .await
    .expect("first create");

    let existing = ingest_service::find_by_body_hash(&pool, profile_id, &body_hash)
        .await
        .expect("dedup check")
        .expect("should find existing");

    assert_eq!(existing.id, first.id);
}

/// Unknown doc_type returns error.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resolve_unknown_doc_type_errors(pool: sqlx::PgPool) {
    let result = ingest_service::resolve_doc_type(&pool, "nonexistent-type").await;
    assert!(result.is_err());
}
