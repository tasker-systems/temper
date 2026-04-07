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
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "mcp-test",
            context_id: context.id,
            doc_type_id,
            title: "MCP Test Resource",
            slug: Some("mcp-test-resource"),
            origin_uri: "mcp://test/create",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
        },
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
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "test",
            context_id: context.id,
            doc_type_id,
            title: "First",
            slug: None,
            origin_uri: "mcp://test/dedup-1",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
        },
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

/// Update resource: manifest body_hash changes and body_updated event is created.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_resource_changes_manifest_body_hash(pool: sqlx::PgPool) {
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

    context_service::create(&pool, profile_id, "update-test")
        .await
        .expect("context create");
    let context = context_service::resolve_by_name(&pool, profile_id, "update-test")
        .await
        .expect("context");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type");

    let original_content = "# Original\n\nOriginal content for update test.";
    let original_hash = format!("sha256:{}", sha2_hex(original_content));
    let empty = serde_json::json!({});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "update-test",
            context_id: context.id,
            doc_type_id,
            title: "Update Test Resource",
            slug: Some("update-test-resource"),
            origin_uri: "mcp://test/update",
            content_hash: &original_hash,
            managed_meta: &empty,
            open_meta: &empty,
        },
    )
    .await
    .expect("create_resource_with_manifest");

    // Now simulate the update flow (same SQL as the MCP tool uses)
    let updated_content = "# Updated\n\nUpdated content after edit.";
    let updated_hash = format!("sha256:{}", sha2_hex(updated_content));
    let managed_hash = ingest_service::hash_json_value(&empty);
    let open_hash = ingest_service::hash_json_value(&empty);

    let mut tx = pool.begin().await.expect("begin tx");

    sqlx::query!(
        "UPDATE kb_resources SET updated = now() WHERE id = $1",
        *resource.id
    )
    .execute(&mut *tx)
    .await
    .expect("update resource timestamp");

    sqlx::query!(
        r#"
        INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
        VALUES ($1, $2, $3, $4, $5, $6, now())
        ON CONFLICT (resource_id)
        DO UPDATE SET body_hash = $2, managed_meta = $3, open_meta = $4,
                      managed_hash = $5, open_hash = $6, updated = now()
        "#,
        *resource.id,
        updated_hash,
        empty,
        empty,
        managed_hash,
        open_hash,
    )
    .execute(&mut *tx)
    .await
    .expect("upsert manifest");

    ingest_service::insert_event_and_audit(
        &mut tx,
        profile_id,
        "mcp",
        context.id,
        resource.id,
        "body_updated",
        "update_body",
        &updated_hash,
        &managed_hash,
        &open_hash,
    )
    .await
    .expect("insert event and audit");

    tx.commit().await.expect("commit tx");

    // Verify manifest has new body_hash
    let manifest_hash: String = sqlx::query_scalar!(
        "SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1",
        *resource.id,
    )
    .fetch_one(&pool)
    .await
    .expect("manifest lookup");
    assert_eq!(manifest_hash, updated_hash);
    assert_ne!(manifest_hash, original_hash);

    // Verify body_updated event was created
    let event_count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_events WHERE resource_id = $1 AND event_type = 'body_updated'",
        *resource.id,
    )
    .fetch_one(&pool)
    .await
    .expect("event count")
    .unwrap_or(0);
    assert_eq!(event_count, 1);
}

/// Context auto-creation: resolve_by_name fails for unknown, create succeeds, then resolve finds it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_auto_creation_for_ingest(pool: sqlx::PgPool) {
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

    // resolve_by_name should fail for non-existent context
    let result = context_service::resolve_by_name(&pool, profile_id, "brand-new-context").await;
    assert!(result.is_err(), "should not find non-existent context");

    // create should succeed
    let created = context_service::create(&pool, profile_id, "brand-new-context")
        .await
        .expect("context creation");
    assert_eq!(created.name, "brand-new-context");

    // resolve_by_name should now find it
    let found = context_service::resolve_by_name(&pool, profile_id, "brand-new-context")
        .await
        .expect("should find created context");
    assert_eq!(found.id, created.id);
}
