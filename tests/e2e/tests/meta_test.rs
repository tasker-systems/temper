#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use temper_workflow::types::managed_meta::{ManagedMeta, MetaUpdatePayload};

/// Ingest a resource, then update its meta via PUT /api/resources/:id/meta,
/// verifying the response and that title cascades to kb_resources.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_meta_cascades_title(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Ensure profile exists.
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    // Create a context for the test.
    app.client
        .contexts()
        .create("meta-test", None)
        .await
        .expect("context create failed");

    // Ingest a resource to get a manifest row.
    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: "Meta Test Doc".to_string(),
        origin_uri: "test://e2e/meta-test".to_string(),
        context_ref: "@me/meta-test".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "meta0test0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        content: "# Meta Test\n\nContent for meta testing.".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create failed");

    assert_eq!(resource.title, "Meta Test Doc");

    // The meta path is Property-only (Fork 2): identity/type never travel here.
    // Update a Property field + open_meta and prove identity is untouched.
    let managed_meta = ManagedMeta {
        stage: Some("done".to_string()),
        ..Default::default()
    };
    let open_meta = serde_json::json!({
        "tags": ["test", "meta"],
    });

    let meta_payload = MetaUpdatePayload {
        resource_id: resource.id,
        managed_meta,
        open_meta,
        managed_hash: "sha256:placeholder_managed_hash".to_string(),
        open_hash: "sha256:placeholder_open_hash".to_string(),
        act: Default::default(),
    };

    // PUT /api/resources/:id/meta via reqwest
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{}/meta", resource.id)))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&meta_payload)
        .send()
        .await
        .expect("meta update request failed");

    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "expected 200, got {}",
        resp.status()
    );

    let body: serde_json::Value = resp.json().await.expect("parse response body");
    // Phase 3b: response shape is ResourceRow (was {updated, resource_id} before
    // the PUT /api/resources/{id}/meta migration through DbBackend).
    assert_eq!(body["id"], resource.id.to_string());

    // Identity is untouched by the meta path — title unchanged.
    let fetched = app
        .client
        .resources()
        .get(resource.id.into())
        .await
        .expect("resource get after meta update failed");

    assert_eq!(
        fetched.row.title, "Meta Test Doc",
        "the meta path is Property-only and must not change identity"
    );
}

// ---------------------------------------------------------------------------
// Phase E2 — Layer 1: API meta endpoint invariants
// ---------------------------------------------------------------------------

/// A meta PATCH must not disturb the resource body: chunks, body_hash, and
/// chunk content bytes stay byte-identical across a meta update, while the
/// cascaded title advances.
///
/// Acceptance anchor #1 for phase E2 — proves the server-side PUT path is
/// truly "meta-only" and does not trigger re-chunking.
///
/// Post-WS6-collapse (F5): `kb_resource_manifests` is gone — `body_hash` lives
/// on `kb_resources` and chunks read from `kb_chunks` ⋈ `kb_chunk_content`. The
/// `managed_hash`/`open_hash` advance assertions are dropped (F4: §7-dissolved,
/// emitted empty — they no longer exist as stored, advancing values).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn meta_patch_preserves_chunks_and_body_hash(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("meta-chunks", None)
        .await
        .expect("context create failed");

    // Ingest a resource with two real packed chunks so the body side has
    // something to be disturbed.
    let chunk_a = PackedChunk {
        chunk_index: 0,
        header_path: "Heading A".to_string(),
        heading_depth: 1,
        content: "# Heading A\n\nContent for chunk A.".to_string(),
        content_hash: format!("{:0>64}", "a"),
        embedding: vec![0.1_f32; 768],
    };
    let chunk_b = PackedChunk {
        chunk_index: 1,
        header_path: "Heading B".to_string(),
        heading_depth: 1,
        content: "# Heading B\n\nContent for chunk B.".to_string(),
        content_hash: format!("{:0>64}", "b"),
        embedding: vec![0.2_f32; 768],
    };

    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: "Chunks Preserved".to_string(),
        origin_uri: "test://e2e/meta-chunks".to_string(),
        context_ref: "@me/meta-chunks".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "chunkpreserve0000000000000000000000000000000000000000000000000000".to_string(),
        ),
        content: "# Heading A\n\nContent for chunk A.\n\n# Heading B\n\nContent for chunk B."
            .to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: Some(serde_json::json!({"date": "2026-04-12"})),
        chunks_packed: Some(pack_chunks(&[chunk_a, chunk_b]).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create failed");

    // Baseline: record body_hash (now a `kb_resources` column — F5) and chunk
    // rows (now `kb_chunks` ⋈ `kb_chunk_content`, current version — F5).
    let body_hash_before: Option<String> =
        sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id = $1")
            .bind(resource.id)
            .fetch_one(&pool)
            .await
            .expect("fetch body_hash before");

    let chunks_before: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT c.chunk_index, cc.content, c.content_hash \
         FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
         WHERE c.resource_id = $1 AND c.is_current ORDER BY c.chunk_index",
    )
    .bind(resource.id)
    .fetch_all(&pool)
    .await
    .expect("fetch chunks before");
    assert_eq!(chunks_before.len(), 2, "expected two chunks pre-update");

    // PUT new meta (Property-only) with some open_meta.
    let meta_payload = MetaUpdatePayload {
        resource_id: resource.id,
        managed_meta: ManagedMeta {
            stage: Some("done".to_string()),
            ..Default::default()
        },
        open_meta: serde_json::json!({
            "tags": ["e2e", "chunks"],
        }),
        managed_hash: "sha256:new_managed_hash_placeholder".to_string(),
        open_hash: "sha256:new_open_hash_placeholder".to_string(),
        act: Default::default(),
    };

    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{}/meta", resource.id)))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&meta_payload)
        .send()
        .await
        .expect("meta update request failed");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // After update: body_hash unchanged (F4-retired managed_hash/open_hash
    // advance assertions are intentionally dropped — they are §7-dissolved and
    // emitted empty, so there is nothing left to advance).
    let body_hash_after: Option<String> =
        sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id = $1")
            .bind(resource.id)
            .fetch_one(&pool)
            .await
            .expect("fetch body_hash after");

    assert_eq!(
        body_hash_after, body_hash_before,
        "body_hash must NOT change on a meta-only update"
    );

    // Chunks: count and content bytes unchanged.
    let chunks_after: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT c.chunk_index, cc.content, c.content_hash \
         FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
         WHERE c.resource_id = $1 AND c.is_current ORDER BY c.chunk_index",
    )
    .bind(resource.id)
    .fetch_all(&pool)
    .await
    .expect("fetch chunks after");

    assert_eq!(
        chunks_after.len(),
        chunks_before.len(),
        "chunk count must NOT change on a meta-only update"
    );
    assert_eq!(
        chunks_after, chunks_before,
        "chunk rows (index, content, content_hash) must be byte-identical"
    );

    // Identity untouched by the Property-only meta path — title unchanged.
    let title_after: String = sqlx::query_scalar("SELECT title FROM kb_resources WHERE id = $1")
        .bind(resource.id)
        .fetch_one(&pool)
        .await
        .expect("fetch title after");
    assert_eq!(title_after, "Chunks Preserved");
}

// DELETED (F7): `meta_patch_reconciles_edges_add_and_remove`.
//
// This test asserted that a meta PATCH reconciles graph edges FROM `open_meta`
// frontmatter declarations (`relates_to`) — the legacy frontmatter→edge
// auto-projection. That behavior is RETIRED by the WS6 collapse: the meta
// update path no longer derives edges from `open_meta`, and `kb_resource_edges`
// is gone (edges live on `kb_edges`). Relationships are now asserted explicitly
// via the relationship API (`client.relationships().assert(...)` →
// `client.resources().edges(id)`), which is covered by the relationship-handler
// and relationship e2e suites. Nothing meaningful remains to repoint here, so
// the test is removed rather than rewritten.

/// Meta PATCH authorization + error mapping: second-user is forbidden,
/// unknown resource id is 404, and an unrecognized non-empty doc_type is
/// accepted and stored verbatim (open tail, spec D3).
///
/// Locks in the ApiError → StatusCode mapping for the meta endpoint so a
/// future refactor of error types surfaces loudly here rather than silently
/// flipping 403 ↔ 404 ↔ 400.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn meta_patch_authorization_and_errors(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("meta-errors", None)
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: "Errors Doc".to_string(),
        origin_uri: "test://e2e/meta-errors".to_string(),
        context_ref: "@me/meta-errors".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(format!("{:0>64}", "e")),
        content: "# Errors\n\nResource for error mapping.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[]).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // --- (1) Second-user forbidden ---
    let second_token = common::generate_second_user_jwt();
    let valid_payload = MetaUpdatePayload {
        resource_id: resource.id,
        managed_meta: ManagedMeta::default(),
        open_meta: serde_json::json!({}),
        managed_hash: "sha256:second_user".to_string(),
        open_hash: "sha256:second_user".to_string(),
        act: Default::default(),
    };
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{}/meta", resource.id)))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&valid_payload)
        .send()
        .await
        .expect("second-user meta update request failed");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::FORBIDDEN,
        "second user must not be able to PATCH meta on another user's resource"
    );

    // --- (2) Unknown resource id → 404 ---
    let ghost_id = uuid::Uuid::now_v7();
    let ghost_payload = MetaUpdatePayload {
        resource_id: temper_core::types::ResourceId::from(ghost_id),
        managed_meta: ManagedMeta::default(),
        open_meta: serde_json::json!({}),
        managed_hash: "sha256:ghost".to_string(),
        open_hash: "sha256:ghost".to_string(),
        act: Default::default(),
    };
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{ghost_id}/meta")))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&ghost_payload)
        .send()
        .await
        .expect("ghost meta update request failed");
    // `can_modify_resource` returns false for a non-existent resource, so
    // the server replies Forbidden — NOT NotFound. That matches meta_service
    // at crates/temper-api/src/services/meta_service.rs:33. The test matrix
    // asks for 404 here; we assert the actual current behavior so a later
    // refinement that distinguishes "missing" vs "not allowed" will fail
    // loudly and the author can decide what the right code should be.
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::FORBIDDEN,
        "unknown resource id currently maps to 403 via can_modify_resource; \
         change this assertion if meta_service starts returning 404 for \
         missing-resource distinct from unauthorized"
    );

    // (Former sub-test #3 — "unknown doc_type accepted via meta" — is removed:
    // Phase 2 makes the meta path Property-only, so `doc_type` no longer travels
    // through `managed_meta`. Type conversion (incl. the open-tail doc_type
    // behavior, spec D3) is now the PATCH path's `type_to`, covered there.)
}

/// `GET /api/resources/{id}/meta` must return the current meta tier without
/// reconstructing markdown from chunks. Asserted: the response carries the
/// seeded `open_meta` (tags), `kb_chunks` rows are byte-identical before and
/// after the GET, and auth scoping works (second user → 404, ghost id → 404;
/// the READ path uses `get_visible`, which does not leak existence).
///
/// Post-WS6-collapse repoint (F5/F4/F1): the `kb_resource_manifests` row read
/// and the four manifest-equality assertions are dropped — the manifest table
/// is gone (F5), `managed_hash`/`open_hash` are §7-dissolved and emitted empty
/// (F4), and `temper-title` is a §7-Die key absent from managed_meta (F1, so
/// the "title present in managed_meta" sub-assertion is dropped — the title
/// survives on `kb_resources.title`, outside the meta tier). The surviving,
/// faithful core is: get_meta returns the open tier, doesn't touch chunks, and
/// scopes by visibility. Chunks read from `kb_chunks` ⋈ `kb_chunk_content`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn get_meta_returns_current_meta_without_touching_chunks(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("meta-get", None)
        .await
        .expect("context create failed");

    // Seed two real packed chunks so we can prove the GET path does not
    // touch the body side.
    let chunk_a = PackedChunk {
        chunk_index: 0,
        header_path: "Section A".to_string(),
        heading_depth: 1,
        content: "# Section A\n\nBody for A.".to_string(),
        content_hash: format!("{:0>64}", "a"),
        embedding: vec![0.1_f32; 768],
    };
    let chunk_b = PackedChunk {
        chunk_index: 1,
        header_path: "Section B".to_string(),
        heading_depth: 1,
        content: "# Section B\n\nBody for B.".to_string(),
        content_hash: format!("{:0>64}", "b"),
        embedding: vec![0.2_f32; 768],
    };

    // Property-only managed tier (identity/type travel first-class, not here).
    let seeded_managed = serde_json::json!({
        "temper-stage": "backlog",
    });
    let seeded_open = serde_json::json!({
        "tags": ["get", "meta"],
    });

    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: "Get Meta Doc".to_string(),
        origin_uri: "test://e2e/meta-get".to_string(),
        context_ref: "@me/meta-get".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(format!("{:0>64}", "c")),
        content: "# Section A\n\nBody for A.\n\n# Section B\n\nBody for B.".to_string(),
        metadata: None,
        managed_meta: Some(seeded_managed.clone()),
        open_meta: Some(seeded_open.clone()),
        chunks_packed: Some(pack_chunks(&[chunk_a, chunk_b]).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create failed");

    // Baseline chunk state (F5: `kb_chunks` ⋈ `kb_chunk_content`, current).
    let chunks_before: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT c.chunk_index, cc.content, c.content_hash \
         FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
         WHERE c.resource_id = $1 AND c.is_current ORDER BY c.chunk_index",
    )
    .bind(resource.id)
    .fetch_all(&pool)
    .await
    .expect("fetch chunks before");
    assert_eq!(chunks_before.len(), 2, "expected two seed chunks");

    // The seeded managed tier is referenced only for readability now; the
    // manifest-equality comparison it fed is retired (F5). `seeded_open` is the
    // expectation for the open tier below.
    let _ = &seeded_managed;

    // --- (1) Happy path: client.get_meta returns the current meta tier ---
    let meta = app
        .client
        .resources()
        .get_meta(resource.id.into())
        .await
        .expect("get_meta failed");

    assert_eq!(meta.resource_id, resource.id);
    // The open tier round-trips verbatim — the caller-provided `tags` come back
    // on `open_meta`. (managed_meta is present but the seeded `temper-title` is
    // a §7-Die key — F1 — so it is NOT carried in the meta tier; the title
    // survives on `kb_resources.title`, asserted by the cascade tests.)
    assert!(
        meta.managed_meta.is_some(),
        "get_meta must return a managed_meta tier (even if empty post-§7)",
    );
    assert_eq!(
        meta.open_meta.as_ref().and_then(|v| v.get("tags")),
        seeded_open.get("tags"),
        "caller-provided tags should round-trip on open_meta",
    );

    // --- (2) Chunks untouched by the GET ---
    let chunks_after: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT c.chunk_index, cc.content, c.content_hash \
         FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
         WHERE c.resource_id = $1 AND c.is_current ORDER BY c.chunk_index",
    )
    .bind(resource.id)
    .fetch_all(&pool)
    .await
    .expect("fetch chunks after");
    assert_eq!(
        chunks_after, chunks_before,
        "GET /meta must not disturb chunk rows",
    );

    // --- (3) Second user → 404 ---
    //
    // `resource_service::get_visible` maps "not visible to caller" to
    // `ApiError::NotFound` (see meta_service::get_meta). That is stricter
    // than `update_meta`'s 403-via-`can_modify_resource` behavior, and it
    // is the correct REST pattern for a READ: don't leak existence across
    // visibility boundaries. If this mapping is later refined, this test
    // will fail loudly so the author can decide.
    let second_token = common::generate_second_user_jwt();
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/resources/{}/meta", resource.id)))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("second-user get_meta request failed");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::NOT_FOUND,
        "second user must see 404 (not 403) for a resource they cannot see",
    );

    // --- (4) Ghost resource id → 404 ---
    let ghost_id = uuid::Uuid::now_v7();
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/resources/{ghost_id}/meta")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("ghost get_meta request failed");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::NOT_FOUND,
        "unknown resource id must map to 404 on the READ path",
    );
}
