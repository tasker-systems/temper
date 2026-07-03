#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ids::{ProfileId, ResourceId};
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, BodyUpdate, Surface, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;

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

/// Helper: count current chunks for a resource.
async fn current_chunk_count(pool: &sqlx::PgPool, id: uuid::Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("chunk count")
}

/// Helper: server-derived body_hash for a resource (collapsed substrate stores
/// it directly on `kb_resources`; the old `kb_resource_manifests` table is gone).
/// Nullable, so `Option`.
async fn body_hash_of(pool: &sqlx::PgPool, id: uuid::Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("body_hash")
}

/// Helper: current chunk rows `(chunk_index, content, content_hash)` ordered by
/// index. The collapsed substrate has no `kb_current_chunks` view, so we join
/// `kb_chunks` to its content side-table `kb_chunk_content`.
async fn current_chunk_rows(pool: &sqlx::PgPool, id: uuid::Uuid) -> Vec<(i32, String, String)> {
    sqlx::query_as(
        "SELECT c.chunk_index, cc.content, c.content_hash \
         FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
         WHERE c.resource_id = $1 AND c.is_current = true ORDER BY c.chunk_index",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .expect("chunk rows")
}

// ---------------------------------------------------------------------------
// Task 28: create resource with chunks and verify they are searchable
// ---------------------------------------------------------------------------

/// Creating a resource with pre-packed chunks (via the production `/api/ingest`
/// path the MCP create tool now uses) stores them in `kb_chunks`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_create_resource_with_markdown_is_searchable(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    app.client
        .contexts()
        .create("round-trip-search", None)
        .await
        .expect("context create");

    let content = "# Concept: Round-Trip Search\n\nThis concept tests that resources with chunks are searchable.";
    let chunks = vec![fake_chunk(
        0,
        "Concept: Round-Trip Search",
        "This concept tests that resources with chunks are searchable.",
    )];
    let packed = temper_core::types::ingest::pack_chunks(&chunks).expect("pack chunks");

    let payload = temper_core::types::ingest::IngestPayload {
        title: "Round-Trip Search Concept".to_string(),
        origin_uri: "mcp://test/round-trip-search".to_string(),
        context_ref: "@me/round-trip-search".to_string(),
        home_cogmap_id: None,
        doc_type_name: "concept".to_string(),
        content_hash: Some(format!("sha256:{}", sha2_hex(content))),
        slug: "round-trip-search-concept".to_string(),
        content: content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(packed),
        act: Default::default(),
    };
    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create");

    // Verify chunks were stored
    let chunk_count = current_chunk_count(&pool, *resource.id).await;

    assert!(
        chunk_count > 0,
        "expected at least 1 chunk for the resource, got {chunk_count}"
    );
}

// ---------------------------------------------------------------------------
// Task 29: validation surfaces structured error for missing required field
// ---------------------------------------------------------------------------

/// Ingesting a task with an out-of-enum temper-stage returns an error and
/// creates no row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_create_resource_schema_validation_surfaces_structured_error(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    app.client
        .contexts()
        .create("validation-test", None)
        .await
        .expect("context create");

    let empty_chunks = temper_core::types::ingest::pack_chunks(&[]).expect("pack empty chunks");

    // Build a task payload with an INVALID temper-stage enum value.
    // (Previously this tested missing temper-stage, but apply_managed_defaults
    // now auto-fills it to "backlog". Testing an invalid enum value still
    // exercises the validation pipeline and error surfacing.)
    let payload = temper_core::types::ingest::IngestPayload {
        title: "Validation Test Task".to_string(),
        origin_uri: "mcp://test/validation".to_string(),
        context_ref: "@me/validation-test".to_string(),
        home_cogmap_id: None,
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
        act: Default::default(),
    };

    // The production ingest path (through the client) rejects the bad enum. The
    // client surfaces the server's error body, so we assert on the contract
    // (create failed AND no row landed) rather than a specific message string.
    let result = app.client.ingest().create(&payload).await;
    assert!(
        result.is_err(),
        "should reject task with invalid temper-stage enum"
    );

    // Verify no resource was created. The collapsed substrate's `kb_resources`
    // no longer carries `slug`/`owner_profile_id`; `origin_uri` uniquely
    // identifies the would-be row.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_resources WHERE origin_uri = $1")
        .bind("mcp://test/validation")
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

/// Creating a resource via the ingest path (which MCP create uses) with
/// pre-packed chunks stores them and makes content retrievable via the
/// substrate read selector `get_content_select`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_ingest_persists_content_as_chunks(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    app.client
        .contexts()
        .create("content-round-trip", None)
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
        context_ref: "@me/content-round-trip".to_string(),
        home_cogmap_id: None,
        doc_type_name: "session".to_string(),
        content_hash: Some(format!("sha256:{}", sha2_hex(&content))),
        slug: "content-round-trip-test".to_string(),
        content,
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(packed),
        act: Default::default(),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest should succeed");

    // Verify chunks were created
    let chunk_count = current_chunk_count(&pool, *resource.id).await;

    assert!(
        chunk_count > 0,
        "ingest() with content should create chunks, got {chunk_count}"
    );

    // Verify content is retrievable via the substrate read selector
    let retrieved = substrate_read::get_content_select(&pool, profile_id, resource.id)
        .await
        .expect("get_content_select");

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
        temper_workflow::schema::validate_frontmatter("task", &yaml_value).expect("schema load");

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
    let summary = temper_mcp::tools::doc_types::build_doc_type_summary("task");

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

/// Update replaces the body and re-chunks atomically: the server-derived
/// body_hash advances and the chunk set is swapped for the new one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_update_resource_changes_content_and_reindexes(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    app.client
        .contexts()
        .create("update-reindex-test", None)
        .await
        .expect("context create");

    // 1. Create resource with 1 chunk via the ingest path
    let original_content = "# Original Research\n\nOriginal content for reindex test.";
    let original_chunks = vec![fake_chunk(
        0,
        "Original Research",
        "Original content for reindex test.",
    )];
    let original_packed =
        temper_core::types::ingest::pack_chunks(&original_chunks).expect("pack original");

    let payload = temper_core::types::ingest::IngestPayload {
        title: "Reindex Test Resource".to_string(),
        origin_uri: "mcp://test/reindex".to_string(),
        context_ref: "@me/update-reindex-test".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(format!("sha256:{}", sha2_hex(original_content))),
        slug: "reindex-test-resource".to_string(),
        content: original_content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(original_packed),
        act: Default::default(),
    };
    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("create resource");

    // Verify 1 initial chunk
    let initial_count = current_chunk_count(&pool, *resource.id).await;
    assert_eq!(initial_count, 1, "expected 1 initial chunk");
    let body_hash_before = body_hash_of(&pool, *resource.id).await;

    // 2. Update with 2 new chunks via the MCP write path (DbBackend, Surface::Mcp)
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
        context_ref: None,
        act: Default::default(),
        origin: Surface::Mcp,
    };
    DbBackend::new(pool.clone(), profile_id)
        .update_resource(cmd)
        .await
        .expect("update via DbBackend");

    // Read back the row via the substrate selector (NOT the retired get_visible).
    let updated_resource = substrate_read::show_select(&pool, profile_id, resource.id)
        .await
        .expect("show_select after update");
    assert_eq!(updated_resource.id, resource.id);

    // 3. Verify the server-derived body_hash advanced (the body changed). The
    // substrate derives body_hash from the chunk hashes rather than echoing the
    // caller-supplied content_hash, so we compare before/after rather than to a
    // literal.
    let body_hash_after = body_hash_of(&pool, *resource.id).await;
    assert_ne!(
        body_hash_after, body_hash_before,
        "body_hash should advance after a content update"
    );

    // 4. Verify the reconstructed body reflects the new content, not the old.
    let reconstructed = substrate_read::get_content_select(&pool, profile_id, resource.id)
        .await
        .expect("get_content_select after update")
        .markdown;
    assert!(
        reconstructed.contains("New section A") && reconstructed.contains("New section B"),
        "reconstructed body should contain the new sections, got: {reconstructed}"
    );
    assert!(
        !reconstructed.contains("Original content for reindex test"),
        "reconstructed body should no longer contain the original content, got: {reconstructed}"
    );

    // 5. Verify the body was reindexed (old chunk retired, new chunk set current).
    // The collapsed update path re-chunks the new body server-side — it ignores the
    // caller-supplied chunks_packed and derives the chunk set from the body block — so
    // the exact chunk count is server-determined, not caller-controlled. The reindex
    // itself is proven above by the advanced body_hash plus the swapped reconstructed
    // content; here we assert the resource is still chunked after the swap.
    let current_count = current_chunk_count(&pool, *resource.id).await;
    assert!(
        current_count > 0,
        "expected the reindexed body to have current chunks, got {current_count}"
    );
}

// ---------------------------------------------------------------------------
// MCP parity: update_resource_meta preserves chunks and body_hash
// ---------------------------------------------------------------------------

/// The MCP `update_resource_meta` tool delegates to the same `DbBackend` write
/// path that powers `PUT /api/resources/{id}/meta`. This locks in the
/// "meta-only" invariants through that entry point: a meta-only update leaves
/// the body (and its derived body_hash) and the chunk rows byte-identical,
/// updates the managed/open frontmatter, and cascades the title to
/// `kb_resources`.
///
/// (The pre-collapse manifest carried separate `managed_hash`/`open_hash`
/// columns whose *advance* was the proxy for "meta changed". Those hashes are
/// §7-dissolved in the substrate, so the proof shifts to asserting the meta
/// *content* actually changed via the `get_meta_select` read selector.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_update_resource_meta_preserves_chunks_and_body_hash(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    app.client
        .contexts()
        .create("mcp-meta-parity", None)
        .await
        .expect("context create");

    // Seed a resource with two real packed chunks.
    let chunk_a = fake_chunk(0, "Section A", "Body for section A.");
    let chunk_b = fake_chunk(1, "Section B", "Body for section B.");
    let content = "# Section A\n\nBody for section A.\n\n# Section B\n\nBody for section B.";
    let packed = temper_core::types::ingest::pack_chunks(&[chunk_a, chunk_b]).expect("pack chunks");

    let payload = temper_core::types::ingest::IngestPayload {
        title: "MCP Meta Parity".to_string(),
        origin_uri: "mcp://test/meta-parity".to_string(),
        context_ref: "@me/mcp-meta-parity".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(format!("sha256:{}", sha2_hex(content))),
        slug: "mcp-meta-parity".to_string(),
        content: content.to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"temper-type": "research"})),
        open_meta: Some(serde_json::json!({"tags": ["mcp", "parity"]})),
        chunks_packed: Some(packed),
        act: Default::default(),
    };
    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("create resource");

    // Baseline chunk rows + derived body_hash.
    let chunks_before = current_chunk_rows(&pool, *resource.id).await;
    assert_eq!(chunks_before.len(), 2, "expected 2 seed chunks");
    let body_hash_before = body_hash_of(&pool, *resource.id).await;

    // Dispatch via DbBackend on Surface::Mcp — the same path
    // tools::resources::update_resource_meta uses in production.
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
        context_ref: None,
        act: Default::default(),
        origin: Surface::Mcp,
    };
    DbBackend::new(pool.clone(), profile_id)
        .update_resource(cmd)
        .await
        .expect("update via DbBackend");

    // Invariant: body_hash unchanged (meta-only update never touches the body).
    let body_hash_after = body_hash_of(&pool, *resource.id).await;
    assert_eq!(
        body_hash_after, body_hash_before,
        "body_hash must NOT change on a meta-only MCP update",
    );

    // Invariant: chunk rows byte-identical through the meta path.
    let chunks_after = current_chunk_rows(&pool, *resource.id).await;
    assert_eq!(
        chunks_after, chunks_before,
        "chunk rows must be byte-identical through the MCP meta path",
    );

    // Invariant: the meta content actually advanced (read via the selector).
    // open_meta round-trips through the meta readback, so its new "updated" tag is the
    // proof the open tier advanced. temper-title is a §7 identity key that maps to the
    // `kb_resources.title` column (NOT the meta property bag), so the managed-title
    // update surfaces via the title cascade asserted below rather than managed_meta.
    let meta = substrate_read::get_meta_select(&pool, profile_id, resource.id)
        .await
        .expect("get_meta_select after update");
    assert!(
        meta.managed_meta.is_some(),
        "managed_meta sourced via get_meta_select",
    );
    let open = meta.open_meta.expect("open_meta present");
    assert_eq!(
        open["tags"][2], "updated",
        "open_meta tags must reflect the meta update",
    );

    // Invariant: title cascaded to kb_resources.
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
/// the MCP path (`DbBackend`, Surface::Mcp) merges per-key: fields the caller
/// omits keep their stored value rather than being wiped. Read back via the
/// `get_meta_select` read selector (the manifest table is gone).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_update_resource_meta_merges_partial_managed_meta(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = resolve_test_profile(&pool).await;

    app.client
        .contexts()
        .create("gap6-merge", None)
        .await
        .expect("context create");

    // Seed a task with several managed fields set.
    let body = "Task body for gap6.";
    let content = format!("# Gap6 Merge Task\n\n{body}");
    let packed = temper_core::types::ingest::pack_chunks(&[fake_chunk(0, "Gap6 Merge Task", body)])
        .expect("pack chunks");
    let payload = temper_core::types::ingest::IngestPayload {
        title: "Gap6 Merge Task".to_string(),
        origin_uri: "mcp://test/gap6".to_string(),
        context_ref: "@me/gap6-merge".to_string(),
        home_cogmap_id: None,
        doc_type_name: "task".to_string(),
        content_hash: Some(format!("sha256:{}", sha2_hex(&content))),
        slug: "gap6-merge-task".to_string(),
        content,
        metadata: None,
        managed_meta: Some(serde_json::json!({
            "temper-type": "task",
            "temper-stage": "in-progress",
            "temper-mode": "build",
            "temper-effort": "large",
        })),
        open_meta: None,
        chunks_packed: Some(packed),
        act: Default::default(),
    };
    let resource = app
        .client
        .ingest()
        .create(&payload)
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
        context_ref: None,
        act: Default::default(),
        origin: Surface::Mcp,
    };
    DbBackend::new(pool.clone(), profile_id)
        .update_resource(cmd)
        .await
        .expect("partial update via DbBackend");

    let meta = substrate_read::get_meta_select(&pool, profile_id, resource.id)
        .await
        .expect("get_meta_select after update");
    let managed = meta.managed_meta.expect("managed_meta present");

    assert_eq!(
        managed.stage.as_deref(),
        Some("done"),
        "the explicitly-updated key must apply",
    );
    assert_eq!(
        managed.mode.as_deref(),
        Some("build"),
        "temper-mode omitted from the call must be preserved",
    );
    assert_eq!(
        managed.effort.as_deref(),
        Some("large"),
        "temper-effort omitted from the call must be preserved",
    );
}

// ---------------------------------------------------------------------------
// Write-side gap 5: meta updates validate against the doc-type schema
// ---------------------------------------------------------------------------

/// A managed_meta update whose merged shape violates the doc-type schema
/// (here: an out-of-enum `temper-stage`) is rejected before any write, and
/// the stored frontmatter is left untouched.
///
/// Locks the restored update-path validation: `DbBackend::update_resource`
/// re-runs the strip → defaults → identity → `validate_managed_meta` pipeline
/// (effective doc_type/context/title taken from the current row), mirroring
/// create. This was "write-side gap 5", regressed by the collapse and restored.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_update_resource_meta_rejects_schema_invalid_field(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = resolve_test_profile(&pool).await;

    app.client
        .contexts()
        .create("gap5-validate", None)
        .await
        .expect("context create");

    let body = "Task body for gap5.";
    let content = format!("# Gap5 Validate Task\n\n{body}");
    let packed =
        temper_core::types::ingest::pack_chunks(&[fake_chunk(0, "Gap5 Validate Task", body)])
            .expect("pack chunks");
    let payload = temper_core::types::ingest::IngestPayload {
        title: "Gap5 Validate Task".to_string(),
        origin_uri: "mcp://test/gap5".to_string(),
        context_ref: "@me/gap5-validate".to_string(),
        home_cogmap_id: None,
        doc_type_name: "task".to_string(),
        content_hash: Some(format!("sha256:{}", sha2_hex(&content))),
        slug: "gap5-validate-task".to_string(),
        content,
        metadata: None,
        managed_meta: Some(serde_json::json!({
            "temper-type": "task",
            "temper-stage": "backlog",
            "temper-mode": "build",
        })),
        open_meta: None,
        chunks_packed: Some(packed),
        act: Default::default(),
    };
    let resource = app
        .client
        .ingest()
        .create(&payload)
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
        context_ref: None,
        act: Default::default(),
        origin: Surface::Mcp,
    };
    let result = DbBackend::new(pool.clone(), profile_id)
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
    let meta = substrate_read::get_meta_select(&pool, profile_id, resource.id)
        .await
        .expect("get_meta_select after rejected update");
    let managed = meta.managed_meta.expect("managed_meta present");
    assert_eq!(
        managed.stage.as_deref(),
        Some("backlog"),
        "a rejected update must leave stored managed_meta untouched",
    );
}

// ---------------------------------------------------------------------------
// WS6 Spec B Task 4: get_resource routes through substrate_read
// ---------------------------------------------------------------------------

/// Drive the production MCP `get_resource` tool fn end-to-end (row via
/// `show_select`, meta via `get_meta_select`, body via `get_content_select`,
/// assembled by `build_enriched`). Proves the contract through the *production
/// caller* (`TemperMcpService` → `require_profile` → `get_resource`): the
/// response carries managed_meta + open_meta off the meta selector, plus a
/// second body part under `include_content`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_get_resource_routes_through_selector_legacy(pool: sqlx::PgPool) {
    use temper_services::config::ApiConfig;
    use temper_services::state::{AppState, JwksKeyStore};

    let app = common::setup(pool.clone()).await;
    // Provision the e2e-test-user profile (auto-created on first profile read).
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    // Seed a resource (owned by the caller) with managed + open meta + a real chunk body.
    app.client
        .contexts()
        .create("selector-route", None)
        .await
        .expect("context create");
    let body = "Selector routing keeps the legacy contract intact.";
    let content = format!("# Selector Route\n\n{body}");
    let packed = temper_core::types::ingest::pack_chunks(&[fake_chunk(0, "Selector Route", body)])
        .expect("pack chunks");
    let payload = temper_core::types::ingest::IngestPayload {
        title: "Selector Route Doc".to_string(),
        origin_uri: "mcp://test/selector-route".to_string(),
        context_ref: "@me/selector-route".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(format!("sha256:{}", sha2_hex(&content))),
        slug: "selector-route-doc".to_string(),
        content,
        metadata: None,
        managed_meta: Some(
            serde_json::json!({"temper-type": "research", "temper-stage": "in-progress"}),
        ),
        open_meta: Some(serde_json::json!({"tags": ["selector", "route"]})),
        chunks_packed: Some(packed),
        act: Default::default(),
    };
    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("create resource");

    // Build an MCP service and seed its profile cache from synthetic JWT claims — the
    // production caller path (`ensure_profile_from_parts` → `require_profile`).
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("decoding key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, jsonwebtoken::Algorithm::RS256);
    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
    };
    let state = AppState::new(pool.clone(), jwks_store, api_config);
    let svc = temper_mcp::service::TemperMcpService::new(state);

    let req = axum::http::Request::builder()
        .extension(temper_services::auth::RawJwtClaims {
            sub: "e2e-test-user".to_string(),
            email: None,
            email_verified: None,
            azp: None,
            gty: None,
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
            iat: 0,
        })
        .body(())
        .expect("build request");
    let (req_parts, ()) = req.into_parts();
    svc.ensure_profile_from_parts(&req_parts)
        .await
        .expect("seed profile cache");

    let result = temper_mcp::tools::resources::get_resource(
        &svc,
        temper_mcp::tools::resources::GetResourceInput {
            id: (*resource.id).to_string(),
            include_content: Some(true),
            fields: None,
        },
    )
    .await
    .expect("get_resource ok");

    // Serialize the CallToolResult to inspect its content parts robustly.
    let v = serde_json::to_value(&result).expect("serialize result");
    let parts = v["content"].as_array().expect("content array");
    assert_eq!(
        parts.len(),
        2,
        "include_content=true yields the enriched json + a body part"
    );
    let enriched: serde_json::Value =
        serde_json::from_str(parts[0]["text"].as_str().expect("first part text"))
            .expect("parse enriched json");
    assert_eq!(
        enriched["doc_type_name"], "research",
        "doc_type_name read off the row"
    );
    assert_eq!(
        enriched["context_name"], "selector-route",
        "context_name read off the row"
    );
    assert!(
        enriched.get("managed_meta").is_some(),
        "managed_meta sourced via get_meta_select"
    );
    assert_eq!(
        enriched["open_meta"]["tags"][0], "selector",
        "open_meta sourced via get_meta_select"
    );
    let body_text = parts[1]["text"].as_str().expect("body part text");
    assert!(
        body_text.contains("Selector routing keeps the legacy contract"),
        "body via get_content_select, got: {body_text}"
    );
}

// ---------------------------------------------------------------------------
// WS6 Spec B Task 6: list_resources routes through list_select + enrich_resources
// ---------------------------------------------------------------------------

/// Drive the production MCP `list_resources` tool fn end-to-end (rows via
/// `substrate_read::list_select` filtered by `context_ref`, enriched per-row
/// via `enrich_resources`). Proves the contract through the *production caller*
/// (`TemperMcpService` → `require_profile` → `list_resources`): the doctype
/// filter narrows the array to matching rows, and every row carries managed_meta
/// + a non-empty context_name.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_list_resources_routes_through_selector_legacy(pool: sqlx::PgPool) {
    use temper_services::config::ApiConfig;
    use temper_services::state::{AppState, JwksKeyStore};

    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    // Seed two resources in one context with distinct doctypes (research + task), each with managed
    // + open meta, so the doctype filter has something to narrow.
    app.client
        .contexts()
        .create("list-selector", None)
        .await
        .expect("context create");

    for (doc_type_name, title, slug, origin_uri) in [
        (
            "research",
            "List Selector Research",
            "list-selector-research",
            "mcp://test/list-selector-research",
        ),
        (
            "task",
            "List Selector Task",
            "list-selector-task",
            "mcp://test/list-selector-task",
        ),
    ] {
        let body = format!("body for {slug}");
        let content = format!("# {title}\n\n{body}");
        let packed = temper_core::types::ingest::pack_chunks(&[fake_chunk(0, title, &body)])
            .expect("pack chunks");
        let managed =
            serde_json::json!({"temper-type": doc_type_name, "temper-stage": "in-progress"});
        let payload = temper_core::types::ingest::IngestPayload {
            title: title.to_string(),
            origin_uri: origin_uri.to_string(),
            context_ref: "@me/list-selector".to_string(),
            home_cogmap_id: None,
            doc_type_name: doc_type_name.to_string(),
            content_hash: Some(format!("sha256:{}", sha2_hex(&content))),
            slug: slug.to_string(),
            content,
            metadata: None,
            managed_meta: Some(managed),
            open_meta: Some(serde_json::json!({"tags": [slug]})),
            chunks_packed: Some(packed),
            act: Default::default(),
        };
        app.client
            .ingest()
            .create(&payload)
            .await
            .expect("create resource");
    }

    // Build an MCP service and seed its profile cache (the production caller path).
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("decoding key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, jsonwebtoken::Algorithm::RS256);
    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
    };
    let state = AppState::new(pool.clone(), jwks_store, api_config);
    let svc = temper_mcp::service::TemperMcpService::new(state);

    let req = axum::http::Request::builder()
        .extension(temper_services::auth::RawJwtClaims {
            sub: "e2e-test-user".to_string(),
            email: None,
            email_verified: None,
            azp: None,
            gty: None,
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
            iat: 0,
        })
        .body(())
        .expect("build request");
    let (req_parts, ()) = req.into_parts();
    svc.ensure_profile_from_parts(&req_parts)
        .await
        .expect("seed profile cache");

    // Filter by doctype=research → only the research row, enriched.
    let result = temper_mcp::tools::resources::list_resources(
        &svc,
        temper_mcp::tools::resources::ListResourcesInput {
            context_ref: Some("@me/list-selector".to_string()),
            doc_type_name: Some("research".to_string()),
            limit: None,
            offset: None,
            fields: None,
        },
    )
    .await
    .expect("list_resources ok");

    let v = serde_json::to_value(&result).expect("serialize result");
    let text = v["content"][0]["text"].as_str().expect("content text");
    let rows: serde_json::Value = serde_json::from_str(text).expect("parse rows array");
    let rows = rows.as_array().expect("rows is an array");
    assert_eq!(
        rows.len(),
        1,
        "doctype=research filter narrows to exactly the one research row"
    );
    let row = &rows[0];
    assert_eq!(
        row["doc_type_name"], "research",
        "filtered row is the research doctype"
    );
    assert_eq!(
        row["context_name"], "list-selector",
        "context_name read off the row"
    );
    assert!(
        row.get("managed_meta").is_some(),
        "managed_meta sourced via enrich_resources (get_meta_batch)"
    );
    assert_eq!(
        row["open_meta"]["tags"][0], "list-selector-research",
        "open_meta sourced via enrich_resources"
    );

    // Unknown doc_type filter → empty result (NOT an error). Pre-collapse the
    // filter resolved a doc-type id first, so an unknown name produced a
    // NotFound that the MCP boundary mapped to invalid_params (the "I1
    // regression" guard). The WS6 collapse folds doc_type filtering into the
    // list SQL as a by-NAME predicate, so an unknown name now simply matches
    // zero rows — the same semantics as any other unmatched filter. That
    // earlier error-mapping behavior is retired; the surviving contract is
    // "unmatched filter yields an empty list, no error".
    let empty = temper_mcp::tools::resources::list_resources(
        &svc,
        temper_mcp::tools::resources::ListResourcesInput {
            context_ref: Some("@me/list-selector".to_string()),
            doc_type_name: Some("no-such-doctype".to_string()),
            limit: None,
            offset: None,
            fields: None,
        },
    )
    .await
    .expect("unknown doc_type filter resolves to an empty list, not an error");
    let v = serde_json::to_value(&empty).expect("serialize result");
    let text = v["content"][0]["text"].as_str().expect("content text");
    let rows: serde_json::Value = serde_json::from_str(text).expect("parse rows array");
    assert_eq!(
        rows.as_array().expect("rows is an array").len(),
        0,
        "an unknown doc_type filter matches zero rows"
    );
}
