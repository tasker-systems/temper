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

/// Helper: build a fake `PackedChunk` with a unit-vector embedding.
fn fake_chunk(index: u32, header: &str, content: &str) -> temper_core::types::ingest::PackedChunk {
    let val = 1.0_f32 / (768.0_f32).sqrt();
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

/// Helper: resolve the e2e test profile from the database.
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

// ---------------------------------------------------------------------------
// Task 28: create resource with chunks and verify they are searchable
// ---------------------------------------------------------------------------

/// Creating a resource with pre-packed chunks stores them in kb_resource_chunks.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_create_resource_with_markdown_is_searchable(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    context_service::create(&pool, profile_id, "round-trip-search")
        .await
        .expect("context create");

    let context = context_service::resolve_by_name(&pool, profile_id, "round-trip-search")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "concept")
        .await
        .expect("doc_type");

    let content = "# Concept: Round-Trip Search\n\nThis concept tests that resources with chunks are searchable.";
    let chunks = vec![fake_chunk(
        0,
        "Concept: Round-Trip Search",
        "This concept tests that resources with chunks are searchable.",
    )];
    let packed = temper_core::types::ingest::pack_chunks(&chunks).expect("pack chunks");
    let body_hash = format!("sha256:{}", sha2_hex(content));
    let empty = serde_json::json!({});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "e2e-round-trip",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "concept",
            title: "Round-Trip Search Concept",
            slug: Some("round-trip-search-concept"),
            origin_uri: "mcp://test/round-trip-search",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
            chunks_packed: Some(&packed),
        },
    )
    .await
    .expect("create_resource_with_manifest");

    // Verify chunks were stored
    let chunk_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true",
    )
    .bind(*resource.id)
    .fetch_one(&pool)
    .await
    .expect("chunk count");

    assert!(
        chunk_count > 0,
        "expected at least 1 chunk for the resource, got {chunk_count}"
    );
}

// ---------------------------------------------------------------------------
// Task 29: validation surfaces structured error for missing required field
// ---------------------------------------------------------------------------

/// Ingesting a task without temper-stage returns a validation error.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_create_resource_schema_validation_surfaces_structured_error(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    // Create the context so ingest can resolve it
    context_service::create(&pool, profile_id, "validation-test")
        .await
        .expect("context create");

    let empty_chunks = temper_core::types::ingest::pack_chunks(&[]).expect("pack empty chunks");

    // Build a task payload with an INVALID temper-stage enum value.
    // (Previously this tested missing temper-stage, but apply_doc_type_defaults
    // now auto-fills it to "backlog". Testing an invalid enum value still
    // exercises the validation pipeline and error detail surfacing.)
    let payload = temper_core::types::ingest::IngestPayload {
        title: "Validation Test Task".to_string(),
        origin_uri: "mcp://test/validation".to_string(),
        context_name: "validation-test".to_string(),
        doc_type_name: "task".to_string(),
        content_hash: Some(format!("sha256:{}", sha2_hex("validation test content"))),
        slug: "validation-test-task".to_string(),
        content: "validation test content".to_string(),
        metadata: None,
        managed_meta: Some(
            serde_json::json!({"temper-stage": "not-a-real-stage", "temper-mode": "build"}),
        ),
        open_meta: None,
        chunks_packed: Some(empty_chunks),
    };

    let result = ingest_service::ingest(&pool, profile_id, "e2e-test-device", payload).await;

    assert!(
        result.is_err(),
        "should reject task with invalid temper-stage enum"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("validation failed") || err_msg.contains("temper-stage"),
        "error should mention validation failure or temper-stage: {err_msg}"
    );

    // Verify no resource was created
    let count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_resources WHERE slug = 'validation-test-task' AND owner_profile_id = $1",
        *profile_id,
    )
    .fetch_one(&pool)
    .await
    .expect("count query")
    .unwrap_or(0);
    assert_eq!(
        count, 0,
        "no resource should be created after validation failure"
    );
}

// ---------------------------------------------------------------------------
// MCP create_resource via ingest() persists content retrievably
// ---------------------------------------------------------------------------

/// Creating a resource via `ingest()` (the path MCP now uses) with pre-packed
/// chunks stores them and makes content retrievable via `get_content`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_ingest_persists_content_as_chunks(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    context_service::create(&pool, profile_id, "content-round-trip")
        .await
        .expect("context create");

    let body = "This session covered the MCP content pipeline fix.";
    let content = format!("# Session Note\n\n{body}");

    // Pre-pack chunks (avoids ONNX model load; pipeline is tested separately)
    let chunks = vec![fake_chunk(0, "Session Note", body)];
    let packed = temper_core::types::ingest::pack_chunks(&chunks).expect("pack chunks");

    let payload = temper_core::types::ingest::IngestPayload {
        title: "Content Round-Trip Test".to_string(),
        origin_uri: "mcp://test/content-round-trip".to_string(),
        context_name: "content-round-trip".to_string(),
        doc_type_name: "session".to_string(),
        content_hash: Some(format!("sha256:{}", sha2_hex(&content))),
        slug: "content-round-trip-test".to_string(),
        content,
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(packed),
    };

    let resource = ingest_service::ingest(&pool, profile_id, "mcp", payload)
        .await
        .expect("ingest should succeed");

    // Verify chunks were created
    let chunk_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true",
    )
    .bind(*resource.id)
    .fetch_one(&pool)
    .await
    .expect("chunk count");

    assert!(
        chunk_count > 0,
        "ingest() with content should create chunks, got {chunk_count}"
    );

    // Verify content is retrievable
    let retrieved =
        temper_api::services::resource_service::get_content(&pool, *profile_id, *resource.id)
            .await
            .expect("get_content");

    assert!(
        !retrieved.is_empty(),
        "get_content should return non-empty string"
    );
    assert!(
        retrieved.contains("MCP content pipeline fix"),
        "retrieved content should contain original text, got: {retrieved}"
    );
}

// ---------------------------------------------------------------------------
// Task 30: describe_doc_type returns usable example
// ---------------------------------------------------------------------------

/// describe_doc_type_impl("task") returns schema, required fields, enum values,
/// and an example_managed_meta that passes validation.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_describe_doc_type_returns_usable_example(_pool: sqlx::PgPool) {
    let response =
        temper_mcp::tools::doc_types::describe_doc_type_impl("task").expect("task is a known type");

    // required_fields contains temper-stage
    assert!(
        response
            .required_fields
            .contains(&"temper-stage".to_string()),
        "required_fields should contain temper-stage: {:?}",
        response.required_fields,
    );

    // enum_fields has temper-stage with backlog
    let stage_enums = response
        .enum_fields
        .get("temper-stage")
        .expect("enum_fields should contain temper-stage");
    assert!(
        stage_enums.contains(&"backlog".to_string()),
        "temper-stage enum values should include backlog: {:?}",
        stage_enums,
    );

    // example_managed_meta round-trips through validate_frontmatter as valid
    // We need to build a full synthetic frontmatter (adding system fields) to validate.
    let example = response.example_managed_meta.clone();
    let mut synthetic = example.as_object().cloned().unwrap_or_default();
    // Inject system-managed fields that the schema requires
    synthetic.insert("slug".to_owned(), serde_json::json!("test-slug"));
    synthetic.insert("title".to_owned(), serde_json::json!("Test Title"));
    synthetic.insert("temper-context".to_owned(), serde_json::json!("test-ctx"));
    synthetic.insert("temper-type".to_owned(), serde_json::json!("task"));
    synthetic.insert(
        "temper-id".to_owned(),
        serde_json::json!("00000000-0000-0000-0000-000000000000"),
    );
    synthetic.insert(
        "temper-created".to_owned(),
        serde_json::json!("2000-01-01T00:00:00Z"),
    );

    let yaml_value: serde_yaml::Value = serde_yaml::to_value(&serde_json::Value::Object(synthetic))
        .expect("JSON to YAML conversion");

    let issues =
        temper_core::schema::validate_frontmatter("task", &yaml_value).expect("schema load");

    assert!(
        issues.is_empty(),
        "example_managed_meta (with system fields) should validate without issues: {:?}",
        issues,
    );
}

// ---------------------------------------------------------------------------
// Task 31: list_doc_types summary includes required_fields
// ---------------------------------------------------------------------------

/// build_doc_type_summary for task has has_schema=true and required_fields with temper-stage.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_list_doc_types_includes_required_fields(_pool: sqlx::PgPool) {
    let summary = temper_mcp::tools::doc_types::build_doc_type_summary(uuid::Uuid::nil(), "task");

    assert!(summary.has_schema, "task should have a schema");
    assert!(
        summary
            .required_fields
            .contains(&"temper-stage".to_string()),
        "task required_fields should include temper-stage: {:?}",
        summary.required_fields,
    );
}

// ---------------------------------------------------------------------------
// Task 32: update resource changes content and reindexes chunks
// ---------------------------------------------------------------------------

/// Update replaces manifest body_hash and chunks atomically.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_update_resource_changes_content_and_reindexes(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    context_service::create(&pool, profile_id, "update-reindex-test")
        .await
        .expect("context create");
    let context = context_service::resolve_by_name(&pool, profile_id, "update-reindex-test")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type");

    // 1. Create resource with 1 chunk
    let original_content = "# Original Research\n\nOriginal content for reindex test.";
    let original_chunks = vec![fake_chunk(
        0,
        "Original Research",
        "Original content for reindex test.",
    )];
    let original_packed =
        temper_core::types::ingest::pack_chunks(&original_chunks).expect("pack original");
    let original_hash = format!("sha256:{}", sha2_hex(original_content));
    let empty = serde_json::json!({});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "e2e-test-device",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: "Reindex Test Resource",
            slug: Some("reindex-test-resource"),
            origin_uri: "mcp://test/reindex",
            content_hash: &original_hash,
            managed_meta: &empty,
            open_meta: &empty,
            chunks_packed: Some(&original_packed),
        },
    )
    .await
    .expect("create resource");

    // Verify 1 initial chunk
    let initial_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true",
    )
    .bind(*resource.id)
    .fetch_one(&pool)
    .await
    .expect("initial chunk count");
    assert_eq!(initial_count, 1, "expected 1 initial chunk");

    // 2. Update with 2 new chunks
    let updated_content = "# Updated Research\n\nNew section A.\nNew section B.";
    let updated_chunks = vec![
        fake_chunk(0, "Updated Research", "New section A."),
        fake_chunk(1, "Updated Research > Section B", "New section B."),
    ];
    let updated_packed =
        temper_core::types::ingest::pack_chunks(&updated_chunks).expect("pack updated");
    let updated_hash = format!("sha256:{}", sha2_hex(updated_content));

    let update_payload = temper_core::types::ingest::IngestPayload {
        title: "Reindex Test Resource".to_string(),
        origin_uri: "mcp://test/reindex".to_string(),
        context_name: "update-reindex-test".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(updated_hash.clone()),
        slug: "reindex-test-resource".to_string(),
        content: updated_content.to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(updated_packed),
    };

    let updated_resource = ingest_service::update(
        &pool,
        profile_id,
        resource.id,
        "e2e-test-device",
        update_payload,
    )
    .await
    .expect("update resource");

    assert_eq!(updated_resource.id, resource.id);

    // 3. Verify manifest body_hash changed
    let manifest_hash: String = sqlx::query_scalar!(
        "SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1",
        *resource.id,
    )
    .fetch_one(&pool)
    .await
    .expect("manifest lookup");

    assert_eq!(manifest_hash, updated_hash, "manifest should have new hash");
    assert_ne!(
        manifest_hash, original_hash,
        "manifest should differ from original"
    );

    // 4. Verify chunks were replaced (old 1 retired, new 2 current)
    let current_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true",
    )
    .bind(*resource.id)
    .fetch_one(&pool)
    .await
    .expect("current chunk count");

    assert_eq!(
        current_count, 2,
        "expected 2 current chunks after update, got {current_count}"
    );
}
