#![cfg(feature = "test-db")]

mod common;

use temper_api::backend::DbBackend;
use temper_api::services::{context_service, event_service, ingest_service, resource_service};
use temper_core::operations::{Backend, BodyUpdate, Surface, UpdateResource};
use temper_core::types::api::EventListParams;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::managed_meta::ManagedMeta;

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
///
/// Runtime query: test-target macros aren't cached by `cargo sqlx prepare`
/// (the test-fixture convention).
async fn resolve_test_profile(pool: &sqlx::PgPool) -> ProfileId {
    let id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_profiles WHERE id IN (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user') LIMIT 1"
    )
    .fetch_one(pool)
    .await
    .expect("profile lookup");
    ProfileId::from(id)
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
            id: ResourceId::new(),
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
    // (Previously this tested missing temper-stage, but apply_managed_defaults
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
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resources WHERE slug = 'validation-test-task' AND owner_profile_id = $1",
    )
    .bind(*profile_id)
    .fetch_one(&pool)
    .await
    .expect("count query");
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
        !retrieved.markdown.is_empty(),
        "get_content should return non-empty markdown"
    );
    assert!(
        retrieved.markdown.contains("MCP content pipeline fix"),
        "retrieved content should contain original text, got: {}",
        retrieved.markdown,
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
    synthetic.insert("temper-slug".to_owned(), serde_json::json!("test-slug"));
    synthetic.insert("temper-title".to_owned(), serde_json::json!("Test Title"));
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

    let yaml_value: serde_yaml::Value = serde_yaml::to_value(serde_json::Value::Object(synthetic))
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
            id: ResourceId::new(),
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

    let cmd = UpdateResource {
        resource: resource.id,
        body: Some(BodyUpdate {
            content: updated_content.to_string(),
            content_hash: Some(updated_hash.clone()),
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

// ---------------------------------------------------------------------------
// MCP parity: update_resource_meta preserves chunks and body_hash
// ---------------------------------------------------------------------------

/// The MCP `update_resource_meta` tool delegates to
/// `meta_service::update_meta`, which is the same service that powers
/// `PUT /api/resources/{id}/meta`. This test locks in the "meta-only"
/// invariants through that entry point so a future refactor that moves
/// MCP tools onto a different service path will fail loudly.
///
/// Mirrors the REST A1 test `meta_patch_preserves_chunks_and_body_hash`
/// in `meta_test.rs`: seed chunks, update meta, assert chunks + body_hash
/// stay byte-identical while managed/open hashes advance and the title
/// cascades to `kb_resources`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_update_resource_meta_preserves_chunks_and_body_hash(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    context_service::create(&pool, profile_id, "mcp-meta-parity")
        .await
        .expect("context create");
    let context = context_service::resolve_by_name(&pool, profile_id, "mcp-meta-parity")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type");

    // Seed a resource with two real packed chunks.
    let chunk_a = fake_chunk(0, "Section A", "Body for section A.");
    let chunk_b = fake_chunk(1, "Section B", "Body for section B.");
    let content = "# Section A\n\nBody for section A.\n\n# Section B\n\nBody for section B.";
    let body_hash = format!("sha256:{}", sha2_hex(content));
    let packed = temper_core::types::ingest::pack_chunks(&[chunk_a, chunk_b]).expect("pack chunks");
    let seeded_managed =
        serde_json::json!({"temper-type": "research", "temper-title": "MCP Meta Parity"});
    let seeded_open = serde_json::json!({"tags": ["mcp", "parity"]});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            id: ResourceId::new(),
            profile_id,
            device_id: "mcp-test",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: "MCP Meta Parity",
            slug: Some("mcp-meta-parity"),
            origin_uri: "mcp://test/meta-parity",
            content_hash: &body_hash,
            managed_meta: &seeded_managed,
            open_meta: &seeded_open,
            chunks_packed: Some(&packed),
        },
    )
    .await
    .expect("create resource");

    // Baseline chunk rows + manifest.
    let chunks_before: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT chunk_index, content, content_hash FROM kb_current_chunks \
         WHERE resource_id = $1 ORDER BY chunk_index",
    )
    .bind(*resource.id)
    .fetch_all(&pool)
    .await
    .expect("chunks before");
    assert_eq!(chunks_before.len(), 2, "expected 2 seed chunks");

    let manifest_before: (String, String, String) = sqlx::query_as(
        "SELECT body_hash, managed_hash, open_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(*resource.id)
    .fetch_one(&pool)
    .await
    .expect("manifest before");

    // Dispatch via DbBackend on Surface::Mcp — the same path tools::resources::
    // update_resource_meta uses in production after the 3c migration.
    let new_managed = ManagedMeta {
        doc_type: Some("research".to_string()),
        title: Some("MCP Meta Parity (updated)".to_string()),
        ..Default::default()
    };
    let new_open = serde_json::json!({"tags": ["mcp", "parity", "updated"]});
    let cmd = UpdateResource {
        resource: ResourceId::from(*resource.id),
        body: None,
        managed_meta: Some(new_managed),
        open_meta: Some(new_open),
        move_to: None,
        origin: Surface::Mcp,
    };
    DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp)
        .update_resource(cmd)
        .await
        .expect("update via DbBackend");

    // Invariants: body_hash unchanged, managed/open hashes advance,
    // chunk rows byte-identical, title cascaded.
    let manifest_after: (String, String, String) = sqlx::query_as(
        "SELECT body_hash, managed_hash, open_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(*resource.id)
    .fetch_one(&pool)
    .await
    .expect("manifest after");

    assert_eq!(
        manifest_after.0, manifest_before.0,
        "body_hash must NOT change on a meta-only MCP update",
    );
    // Phase 5: server now recomputes managed_hash and open_hash on meta
    // updates rather than trusting caller-supplied values, so the assertion
    // shifts from "matches the payload" to "is the canonical server hash"
    // and "advanced from the pre-update value".
    assert_ne!(
        manifest_after.1, manifest_before.1,
        "managed_hash must advance from its pre-update value",
    );
    assert!(
        manifest_after.1.starts_with("sha256:"),
        "managed_hash must be a server-computed sha256 hash; got {}",
        manifest_after.1,
    );
    assert_ne!(
        manifest_after.2, manifest_before.2,
        "open_hash must advance from its pre-update value",
    );
    assert!(
        manifest_after.2.starts_with("sha256:"),
        "open_hash must be a server-computed sha256 hash; got {}",
        manifest_after.2,
    );

    let chunks_after: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT chunk_index, content, content_hash FROM kb_current_chunks \
         WHERE resource_id = $1 ORDER BY chunk_index",
    )
    .bind(*resource.id)
    .fetch_all(&pool)
    .await
    .expect("chunks after");
    assert_eq!(
        chunks_after, chunks_before,
        "chunk rows must be byte-identical through the MCP meta path",
    );

    let title_after: String = sqlx::query_scalar("SELECT title FROM kb_resources WHERE id = $1")
        .bind(*resource.id)
        .fetch_one(&pool)
        .await
        .expect("title after");
    assert_eq!(
        title_after, "MCP Meta Parity (updated)",
        "title must cascade from managed_meta on the MCP path",
    );
}

// ---------------------------------------------------------------------------
// Write-side gap 6: partial managed_meta updates merge per-key
// ---------------------------------------------------------------------------

/// Regression guard for the PATCH-not-PUT semantics confirmed during the
/// 2026-05-21 write-side gap spike. A partial `managed_meta` update through
/// the MCP path (`DbBackend` -> `resource_service::update`) merges per-key:
/// fields the caller omits keep their stored value rather than being wiped.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_update_resource_meta_merges_partial_managed_meta(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = resolve_test_profile(&pool).await;

    context_service::create(&pool, profile_id, "gap6-merge")
        .await
        .expect("context create");
    let context = context_service::resolve_by_name(&pool, profile_id, "gap6-merge")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "task")
        .await
        .expect("doc_type");

    // Seed a task with several managed fields set.
    let seeded_managed = serde_json::json!({
        "temper-type": "task",
        "temper-stage": "in-progress",
        "temper-mode": "build",
        "temper-effort": "large",
    });
    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            id: ResourceId::new(),
            profile_id,
            device_id: "mcp-test",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "task",
            title: "Gap6 Merge Task",
            slug: Some("gap6-merge-task"),
            origin_uri: "mcp://test/gap6",
            content_hash: "",
            managed_meta: &seeded_managed,
            open_meta: &serde_json::json!({}),
            chunks_packed: None,
        },
    )
    .await
    .expect("create resource");

    // Partial update: change ONLY the stage.
    let cmd = UpdateResource {
        resource: ResourceId::from(*resource.id),
        body: None,
        managed_meta: Some(ManagedMeta {
            stage: Some("done".to_string()),
            ..Default::default()
        }),
        open_meta: None,
        move_to: None,
        origin: Surface::Mcp,
    };
    DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp)
        .update_resource(cmd)
        .await
        .expect("partial update via DbBackend");

    let managed: serde_json::Value =
        sqlx::query_scalar("SELECT managed_meta FROM kb_resource_manifests WHERE resource_id = $1")
            .bind(*resource.id)
            .fetch_one(&pool)
            .await
            .expect("managed_meta after");

    assert_eq!(
        managed["temper-stage"], "done",
        "the explicitly-updated key must apply",
    );
    assert_eq!(
        managed["temper-mode"], "build",
        "temper-mode omitted from the call must be preserved",
    );
    assert_eq!(
        managed["temper-effort"], "large",
        "temper-effort omitted from the call must be preserved",
    );
}

// ---------------------------------------------------------------------------
// list_events surfaces changed-key deltas, not just hashes
// ---------------------------------------------------------------------------

/// `list_events` payloads expose *which* managed/open keys changed in an
/// update event, not just the rollup hash. Acceptance criterion for
/// `2026-05-19-mcp-list-events-surface-payload-deltas-beyond-hashes`:
/// change one managed_meta key, then confirm the `managed_meta_updated`
/// event identifies that key.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_events_managed_meta_update_surfaces_changed_keys(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = resolve_test_profile(&pool).await;

    context_service::create(&pool, profile_id, "list-events-delta")
        .await
        .expect("context create");
    let context = context_service::resolve_by_name(&pool, profile_id, "list-events-delta")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "task")
        .await
        .expect("doc_type");

    let seeded_managed = serde_json::json!({
        "temper-type": "task",
        "temper-stage": "in-progress",
        "temper-mode": "build",
    });
    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            id: ResourceId::new(),
            profile_id,
            device_id: "mcp-test",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "task",
            title: "List Events Delta Task",
            slug: Some("list-events-delta-task"),
            origin_uri: "mcp://test/list-events-delta",
            content_hash: "",
            managed_meta: &seeded_managed,
            open_meta: &serde_json::json!({}),
            chunks_packed: None,
        },
    )
    .await
    .expect("create resource");

    // Change exactly one managed_meta key: temper-stage.
    let cmd = UpdateResource {
        resource: ResourceId::from(*resource.id),
        body: None,
        managed_meta: Some(ManagedMeta {
            stage: Some("done".to_string()),
            ..Default::default()
        }),
        open_meta: None,
        move_to: None,
        origin: Surface::Mcp,
    };
    DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp)
        .update_resource(cmd)
        .await
        .expect("update via DbBackend");

    // list_events (the MCP tool delegates to event_service::list_visible).
    let events = event_service::list_visible(
        &pool,
        *profile_id,
        EventListParams {
            resource_id: Some(*resource.id),
            event_type: Some("managed_meta_updated".to_string()),
            limit: Some(10),
            offset: None,
        },
    )
    .await
    .expect("list events");

    let event = events
        .first()
        .expect("a managed_meta_updated event was recorded");
    let changed: Vec<&str> = event
        .payload
        .get("managed_keys_changed")
        .and_then(|v| v.as_array())
        .expect("payload carries managed_keys_changed")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        changed.contains(&"temper-stage"),
        "changed-key set must identify the updated key, got: {changed:?}",
    );
    // The base hash rollup is still present alongside the delta.
    assert!(
        event.payload.get("managed_hash").is_some(),
        "base hash rollup must remain in the payload",
    );
}

// ---------------------------------------------------------------------------
// Write-side gap 5: meta updates validate against the doc-type schema
// ---------------------------------------------------------------------------

/// A managed_meta update whose merged shape violates the doc-type schema
/// (here: an out-of-enum `temper-stage`) is rejected before any write, and
/// the stored frontmatter is left untouched. Closes the gap where
/// `resource_service::update` applied doc-type defaults but never ran the
/// schema validation the create/ingest path enforces.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_update_resource_meta_rejects_schema_invalid_field(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = resolve_test_profile(&pool).await;

    context_service::create(&pool, profile_id, "gap5-validate")
        .await
        .expect("context create");
    let context = context_service::resolve_by_name(&pool, profile_id, "gap5-validate")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "task")
        .await
        .expect("doc_type");

    let seeded_managed = serde_json::json!({
        "temper-type": "task",
        "temper-stage": "backlog",
        "temper-mode": "build",
    });
    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            id: ResourceId::new(),
            profile_id,
            device_id: "mcp-test",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "task",
            title: "Gap5 Validate Task",
            slug: Some("gap5-validate-task"),
            origin_uri: "mcp://test/gap5",
            content_hash: "",
            managed_meta: &seeded_managed,
            open_meta: &serde_json::json!({}),
            chunks_packed: None,
        },
    )
    .await
    .expect("create resource");

    // Update with a temper-stage value outside the task schema's enum.
    let cmd = UpdateResource {
        resource: ResourceId::from(*resource.id),
        body: None,
        managed_meta: Some(ManagedMeta {
            stage: Some("not-a-real-stage".to_string()),
            ..Default::default()
        }),
        open_meta: None,
        move_to: None,
        origin: Surface::Mcp,
    };
    let result = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp)
        .update_resource(cmd)
        .await;

    assert!(
        result.is_err(),
        "an out-of-enum temper-stage must be rejected by schema validation",
    );
    let err = format!("{:?}", result.unwrap_err());
    assert!(
        err.contains("temper-stage") || err.to_lowercase().contains("validation"),
        "error should surface the schema validation failure: {err}",
    );

    // The rejected update must not have mutated the stored frontmatter.
    let managed: serde_json::Value =
        sqlx::query_scalar("SELECT managed_meta FROM kb_resource_manifests WHERE resource_id = $1")
            .bind(*resource.id)
            .fetch_one(&pool)
            .await
            .expect("managed_meta after");
    assert_eq!(
        managed["temper-stage"], "backlog",
        "a rejected update must leave stored managed_meta untouched",
    );
}
