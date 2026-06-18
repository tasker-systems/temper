#![cfg(feature = "test-db")]

mod common;

use temper_api::backend::DbBackend;
use temper_api::services::{context_service, ingest_service, resource_service};
use temper_core::operations::{Backend, BodyUpdate, Surface, UpdateResource};
use temper_core::types::ids::{ProfileId, ResourceId};

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
            id: ResourceId::new(),
            profile_id,
            device_id: "mcp-test",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: "MCP Test Resource",
            slug: Some("mcp-test-resource"),
            origin_uri: "mcp://test/create",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
            chunks_packed: None,
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
        "SELECT count(*) FROM kb_events WHERE resource_id = $1 AND event_type_id = (SELECT id FROM kb_event_types WHERE name = 'resource_created')",
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
            id: ResourceId::new(),
            profile_id,
            device_id: "test",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: "First",
            slug: None,
            origin_uri: "mcp://test/dedup-1",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
            chunks_packed: None,
        },
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
            id: ResourceId::new(),
            profile_id,
            device_id: "update-test",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: "Update Test Resource",
            slug: Some("update-test-resource"),
            origin_uri: "mcp://test/update",
            content_hash: &original_hash,
            managed_meta: &empty,
            open_meta: &empty,
            chunks_packed: None,
        },
    )
    .await
    .expect("create_resource_with_manifest");

    // Now simulate the update flow (same SQL as the MCP tool uses)
    let updated_content = "# Updated\n\nUpdated content after edit.";
    let updated_hash = format!("sha256:{}", sha2_hex(updated_content));
    let managed_hash = temper_core::hash::compute_managed_hash("research", &empty);
    let open_hash = temper_core::hash::compute_open_hash(&empty);

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
        ingest_service::InsertEventAndAuditParams {
            profile_id,
            device_id: "mcp",
            context_id: context.id,
            resource_id: resource.id,
            event_type: "body_updated",
            action: "update_body",
            body_hash: &updated_hash,
            managed_hash: &managed_hash,
            open_hash: &open_hash,
            payload_extra: None,
        },
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
        "SELECT count(*) FROM kb_events WHERE resource_id = $1 AND event_type_id = (SELECT id FROM kb_event_types WHERE name = 'body_updated')",
        *resource.id,
    )
    .fetch_one(&pool)
    .await
    .expect("event count")
    .unwrap_or(0);
    assert_eq!(event_count, 1);
}

/// Helper: build a fake `PackedChunk` with a zero-vector embedding.
fn fake_chunk(index: u32, header: &str, content: &str) -> temper_core::types::ingest::PackedChunk {
    // Use a non-zero embedding: a unit vector with 1/sqrt(768) in each dimension
    let val = 1.0_f32 / (768.0_f32).sqrt();
    // content_hash column is VARCHAR(64), so use a short hash (hex only, no prefix)
    let hash = &sha2_hex(content)[..16];
    temper_core::types::ingest::PackedChunk {
        chunk_index: index,
        header_path: header.to_string(),
        heading_depth: 0,
        content: content.to_string(),
        content_hash: hash.to_string(),
        embedding: vec![val; 768],
    }
}

/// Update replaces chunks atomically: update with new pre-packed chunks replaces old ones.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_resource_from_markdown_replaces_chunks(pool: sqlx::PgPool) {
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

    context_service::create(&pool, profile_id, "chunk-update-test")
        .await
        .expect("context create");

    let context = context_service::resolve_by_name(&pool, profile_id, "chunk-update-test")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type");

    // 1. Create a resource with 1 pre-packed chunk
    let original_content = "Original content for chunk update testing.";
    let original_chunks = vec![fake_chunk(0, "Original Doc", original_content)];
    let original_packed =
        temper_core::types::ingest::pack_chunks(&original_chunks).expect("pack original chunks");
    let original_hash = format!("sha256:{}", sha2_hex(original_content));
    let empty = serde_json::json!({});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            id: ResourceId::new(),
            profile_id,
            device_id: "e2e-test-device",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: "Chunk Update Test",
            slug: Some("chunk-update-test"),
            origin_uri: "mcp://test/chunk-update",
            content_hash: &original_hash,
            managed_meta: &empty,
            open_meta: &empty,
            chunks_packed: Some(&original_packed),
        },
    )
    .await
    .expect("create resource with chunks");

    // Verify initial chunks exist (1 current chunk)
    let initial_chunk_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true",
    )
    .bind(*resource.id)
    .fetch_one(&pool)
    .await
    .expect("initial chunk count");
    assert_eq!(initial_chunk_count, 1, "expected 1 initial chunk");

    // 2. Update the resource via update() with 3 different pre-packed chunks
    let updated_content = "New section B.\nSection C content.\nExtra trailing chunk.";
    let updated_chunks = vec![
        fake_chunk(0, "Updated Doc", "New section B."),
        fake_chunk(1, "Updated Doc > Section C", "Section C content."),
        fake_chunk(2, "Updated Doc", "Extra trailing chunk."),
    ];
    let updated_packed =
        temper_core::types::ingest::pack_chunks(&updated_chunks).expect("pack updated chunks");

    let cmd = UpdateResource {
        resource: ResourceId::from(*resource.id),
        body: Some(BodyUpdate {
            content: updated_content.to_string(),
            content_hash: Some(format!("sha256:{}", sha2_hex(updated_content))),
            chunks_packed: Some(updated_packed),
        }),
        managed_meta: Some(
            serde_json::from_value(serde_json::json!({"date": "2026-04-10"}))
                .expect("managed_meta"),
        ),
        open_meta: None,
        move_to: None,
        origin: Surface::Mcp,
    };
    DbBackend::new(
        pool.clone(),
        profile_id,
        "e2e-test-device".to_string(),
        Surface::Mcp,
    )
    .update_resource(cmd)
    .await
    .expect("update via DbBackend");

    let updated_resource = resource_service::get_visible(&pool, *profile_id, *resource.id)
        .await
        .expect("get_visible after update");
    assert_eq!(updated_resource.id, resource.id);

    // 3. Verify chunks were atomically replaced: old 1 chunk retired, new 3 current chunks
    let current_chunk_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true",
    )
    .bind(*resource.id)
    .fetch_one(&pool)
    .await
    .expect("current chunk count");
    assert_eq!(
        current_chunk_count, 3,
        "expected 3 current chunks after update, got {current_chunk_count}"
    );
}

/// Precomputed-path dispatch: chunks_packed provided → stored verbatim, no re-computation.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_resource_dispatches_on_chunks_packed_presence(pool: sqlx::PgPool) {
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

    context_service::create(&pool, profile_id, "precomputed-test")
        .await
        .expect("context create");

    let context = context_service::resolve_by_name(&pool, profile_id, "precomputed-test")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type");

    // Build pre-computed chunks with known content_hash values.
    let chunks = vec![
        fake_chunk(0, "Section A", "First precomputed chunk content."),
        fake_chunk(1, "Section B", "Second precomputed chunk content."),
        fake_chunk(2, "Section C", "Third precomputed chunk content."),
    ];
    let expected_hashes: Vec<String> = chunks.iter().map(|c| c.content_hash.clone()).collect();
    let packed = temper_core::types::ingest::pack_chunks(&chunks).expect("pack chunks");

    let content = "First precomputed chunk content.\nSecond precomputed chunk content.\nThird precomputed chunk content.";
    let body_hash = format!("sha256:{}", sha2_hex(content));
    let empty = serde_json::json!({});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            id: ResourceId::new(),
            profile_id,
            device_id: "precomputed-test-device",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: "Precomputed Chunks Test",
            slug: Some("precomputed-chunks-test"),
            origin_uri: "mcp://test/precomputed",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
            chunks_packed: Some(&packed),
        },
    )
    .await
    .expect("create_resource_with_manifest");

    // Query stored chunks and verify content_hash values match exactly what was sent.
    let stored_chunks = sqlx::query!(
        "SELECT content_hash, chunk_index FROM kb_chunks WHERE resource_id = $1 AND is_current = true ORDER BY chunk_index",
        *resource.id
    )
    .fetch_all(&pool)
    .await
    .expect("chunk lookup");

    assert_eq!(
        stored_chunks.len(),
        3,
        "expected 3 stored chunks, got {}",
        stored_chunks.len()
    );

    for (i, stored) in stored_chunks.iter().enumerate() {
        assert_eq!(
            stored.content_hash, expected_hashes[i],
            "chunk {i} content_hash mismatch: server must store precomputed chunks verbatim"
        );
    }
}

/// Tier-1 fields stripped: agent-supplied temper-id/temper-created/temper-owner are removed from managed_meta.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_resource_strips_tier1_fields_from_managed_meta(pool: sqlx::PgPool) {
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

    context_service::create(&pool, profile_id, "strip-test")
        .await
        .expect("context create");

    let fake_agent_id = "00000000-0000-0000-0000-000000000001";
    // Include tier-1 fields (to be stripped) alongside valid tier-3 fields.
    // After stripping, {"date": "2026-04-10"} remains, which satisfies the
    // research schema's required "date" field.
    let managed_meta = serde_json::json!({
        "temper-id": fake_agent_id,
        "temper-created": "2020-01-01",
        "temper-owner": "@someone-else",
        "date": "2026-04-10",
    });

    let empty_chunks = temper_core::types::ingest::pack_chunks(&[]).expect("pack empty chunks");
    let content = "# Strip Test\n\nContent for tier-1 field stripping test.";
    let content_hash = format!("sha256:{}", sha2_hex(content));

    let payload = temper_core::types::ingest::IngestPayload {
        title: "Strip Tier-1 Test".to_string(),
        origin_uri: "mcp://test/strip-tier1".to_string(),
        context_name: "strip-test".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(content_hash),
        slug: "strip-tier1-test".to_string(),
        content: content.to_string(),
        metadata: None,
        managed_meta: Some(managed_meta),
        open_meta: None,
        chunks_packed: Some(empty_chunks),
    };

    let resource = ingest_service::ingest(&pool, profile_id, "e2e-test-device", payload)
        .await
        .expect("ingest should succeed despite tier-1 fields in managed_meta");

    // The server-generated ID must not be the one the agent tried to inject
    assert_ne!(
        resource.id.to_string(),
        fake_agent_id,
        "server must not use agent-supplied temper-id"
    );

    // Verify tier-1 fields were stripped from the stored managed_meta
    let stored_managed_meta: serde_json::Value = sqlx::query_scalar!(
        "SELECT managed_meta FROM kb_resource_manifests WHERE resource_id = $1",
        *resource.id,
    )
    .fetch_one(&pool)
    .await
    .expect("manifest lookup");

    let obj = stored_managed_meta
        .as_object()
        .expect("managed_meta is object");
    assert!(
        !obj.contains_key("temper-id"),
        "temper-id must be stripped from stored managed_meta"
    );
    assert!(
        !obj.contains_key("temper-created"),
        "temper-created must be stripped from stored managed_meta"
    );
    assert!(
        !obj.contains_key("temper-owner"),
        "temper-owner must be stripped from stored managed_meta"
    );
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

/// Update rejects tier-2 structural fields (temper-context, temper-type) in managed_meta.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_resource_rejects_tier2_fields_in_managed_meta(pool: sqlx::PgPool) {
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

    context_service::create(&pool, profile_id, "tier2-reject-test")
        .await
        .expect("context create");

    let context = context_service::resolve_by_name(&pool, profile_id, "tier2-reject-test")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type");

    // Create a resource first
    let content = "# Tier-2 Test\n\nContent for tier-2 rejection test.";
    let body_hash = format!("sha256:{}", sha2_hex(content));
    let empty = serde_json::json!({});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            id: ResourceId::new(),
            profile_id,
            device_id: "e2e-test-device",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: "Tier-2 Rejection Test",
            slug: Some("tier2-rejection-test"),
            origin_uri: "mcp://test/tier2-reject",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
            chunks_packed: None,
        },
    )
    .await
    .expect("create resource");

    // Attempt body+meta update that tries to change context — must be rejected
    // as a structural move. The check lives in resource_service::update; the
    // message format mirrors the original IngestError::StructuralMoveNotSupported.
    let empty_chunks = temper_core::types::ingest::pack_chunks(&[]).expect("pack empty chunks");
    let cmd = UpdateResource {
        resource: ResourceId::from(*resource.id),
        body: Some(BodyUpdate {
            content: content.to_string(),
            content_hash: Some(body_hash.clone()),
            chunks_packed: Some(empty_chunks),
        }),
        managed_meta: Some(
            serde_json::from_value(serde_json::json!({"temper-context": "other-context"}))
                .expect("managed_meta"),
        ),
        open_meta: None,
        move_to: None,
        origin: Surface::Mcp,
    };
    let result = DbBackend::new(
        pool.clone(),
        profile_id,
        "e2e-test-device".to_string(),
        Surface::Mcp,
    )
    .update_resource(cmd)
    .await;

    assert!(
        result.is_err(),
        "should reject tier-2 field in managed_meta"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("temper-context") || err_msg.contains("structural move"),
        "error should mention the field or structural move: {err_msg}"
    );
}
