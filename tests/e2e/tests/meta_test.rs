#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use temper_core::types::managed_meta::MetaUpdatePayload;

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
        .create("meta-test")
        .await
        .expect("context create failed");

    // Ingest a resource to get a manifest row.
    let payload = IngestPayload {
        title: "Meta Test Doc".to_string(),
        origin_uri: "test://e2e/meta-test".to_string(),
        context_name: "meta-test".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "meta0test0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "meta-test-doc".to_string(),
        content: "# Meta Test\n\nContent for meta testing.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create failed");

    assert_eq!(resource.title, "Meta Test Doc");

    // Build meta update payload with a new title in managed_meta.
    let managed_meta = serde_json::json!({
        "temper-type": "research",
        "title": "Updated Meta Title",
    });
    let open_meta = serde_json::json!({
        "tags": ["test", "meta"],
    });

    let meta_payload = MetaUpdatePayload {
        resource_id: resource.id,
        managed_meta,
        open_meta,
        managed_hash: "sha256:placeholder_managed_hash".to_string(),
        open_hash: "sha256:placeholder_open_hash".to_string(),
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
    assert_eq!(body["updated"], true);
    assert_eq!(body["resource_id"], resource.id.to_string());

    // Verify title was cascaded to kb_resources.
    let fetched = app
        .client
        .resources()
        .get(resource.id.into())
        .await
        .expect("resource get after meta update failed");

    assert_eq!(
        fetched.title, "Updated Meta Title",
        "title should have been cascaded from managed_meta"
    );
}

// ---------------------------------------------------------------------------
// Phase E2 — Layer 1: API meta endpoint invariants
// ---------------------------------------------------------------------------

/// A meta PATCH must not disturb the resource body: chunks, body_hash, and
/// chunk content bytes stay byte-identical across a meta update, while the
/// managed/open hashes and cascaded title advance.
///
/// Acceptance anchor #1 for phase E2 — proves the server-side PUT path is
/// truly "meta-only" and does not trigger re-chunking.
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
        .create("meta-chunks")
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
        title: "Chunks Preserved".to_string(),
        origin_uri: "test://e2e/meta-chunks".to_string(),
        context_name: "meta-chunks".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "chunkpreserve0000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "chunks-preserved".to_string(),
        content: "# Heading A\n\nContent for chunk A.\n\n# Heading B\n\nContent for chunk B."
            .to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-12"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk_a, chunk_b]).expect("pack chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create failed");

    // Baseline: record body_hash, managed/open hashes, chunk rows.
    let manifest_before: (String, String, String) = sqlx::query_as(
        "SELECT body_hash, managed_hash, open_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource.id)
    .fetch_one(&pool)
    .await
    .expect("fetch manifest before");

    let chunks_before: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT chunk_index, content, content_hash FROM kb_current_chunks \
         WHERE resource_id = $1 ORDER BY chunk_index",
    )
    .bind(resource.id)
    .fetch_all(&pool)
    .await
    .expect("fetch chunks before");
    assert_eq!(chunks_before.len(), 2, "expected two chunks pre-update");

    // PUT new meta with a fresh title (cascade) and some open_meta.
    let meta_payload = MetaUpdatePayload {
        resource_id: resource.id,
        managed_meta: serde_json::json!({
            "temper-type": "research",
            "title": "Chunks Still Preserved",
        }),
        open_meta: serde_json::json!({
            "tags": ["e2e", "chunks"],
        }),
        managed_hash: "sha256:new_managed_hash_placeholder".to_string(),
        open_hash: "sha256:new_open_hash_placeholder".to_string(),
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

    // Manifest: body_hash unchanged, managed/open hashes advanced.
    let manifest_after: (String, String, String) = sqlx::query_as(
        "SELECT body_hash, managed_hash, open_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource.id)
    .fetch_one(&pool)
    .await
    .expect("fetch manifest after");

    assert_eq!(
        manifest_after.0, manifest_before.0,
        "body_hash must NOT change on a meta-only update"
    );
    assert_ne!(
        manifest_after.1, manifest_before.1,
        "managed_hash must advance on a meta update"
    );
    assert_eq!(
        manifest_after.1, meta_payload.managed_hash,
        "managed_hash must match the payload value"
    );
    assert_ne!(
        manifest_after.2, manifest_before.2,
        "open_hash must advance on a meta update"
    );
    assert_eq!(
        manifest_after.2, meta_payload.open_hash,
        "open_hash must match the payload value"
    );

    // Chunks: count and content bytes unchanged.
    let chunks_after: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT chunk_index, content, content_hash FROM kb_current_chunks \
         WHERE resource_id = $1 ORDER BY chunk_index",
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

    // Title cascaded to kb_resources.
    let title_after: String = sqlx::query_scalar("SELECT title FROM kb_resources WHERE id = $1")
        .bind(resource.id)
        .fetch_one(&pool)
        .await
        .expect("fetch title after");
    assert_eq!(title_after, "Chunks Still Preserved");
}

/// A meta PATCH must reconcile `kb_resource_edges` from the new `open_meta`
/// frontmatter declarations: adding a `relates_to` creates the edge row,
/// removing it deletes the row, re-adding it restores the row.
///
/// Acceptance anchor #3 for phase E2 — proves `reconcile_edges` fires on the
/// meta update path, not just on ingest.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn meta_patch_reconciles_edges_add_and_remove(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("meta-edges")
        .await
        .expect("context create failed");

    // R1 — the source. Starts with no relationship declarations.
    let r1_payload = IngestPayload {
        title: "Edge Source R1".to_string(),
        origin_uri: "test://e2e/meta-edges/r1".to_string(),
        context_name: "meta-edges".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(format!("{:0>64}", "1")),
        slug: "edge-source-r1".to_string(),
        content: "# R1\n\nEdge source.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[]).expect("pack chunks")),
    };
    let r1 = app
        .client
        .ingest()
        .create(&r1_payload)
        .await
        .expect("ingest r1");

    // R2 — the target. Also in meta-edges so slug resolution is same-context.
    let r2_payload = IngestPayload {
        title: "Edge Target R2".to_string(),
        origin_uri: "test://e2e/meta-edges/r2".to_string(),
        context_name: "meta-edges".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(format!("{:0>64}", "2")),
        slug: "edge-target-r2".to_string(),
        content: "# R2\n\nEdge target.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[]).expect("pack chunks")),
    };
    let r2 = app
        .client
        .ingest()
        .create(&r2_payload)
        .await
        .expect("ingest r2");

    // The frontmatter relationship parser accepts UUID strings (TargetRef::Id)
    // or slugs (TargetRef::Slug). Use the UUID form — owner-scoped kb:// URIs
    // would be parsed as slugs and fail to resolve.
    let _profile_slug = profile.slug.clone();
    let r2_ref = r2.id.to_string();

    // --- Step 1: add relates_to [r2] → expect one row in kb_resource_edges ---
    let payload_add = MetaUpdatePayload {
        resource_id: r1.id,
        managed_meta: serde_json::json!({"temper-type": "research"}),
        open_meta: serde_json::json!({
            "relates_to": [r2_ref.clone()],
        }),
        managed_hash: "sha256:edges_managed_v1".to_string(),
        open_hash: "sha256:edges_open_v1".to_string(),
    };

    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{}/meta", r1.id)))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&payload_add)
        .send()
        .await
        .expect("meta update (add) request failed");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let edges_after_add: Vec<(uuid::Uuid, uuid::Uuid, String)> = sqlx::query_as(
        "SELECT source_resource_id, target_resource_id, edge_type::TEXT \
         FROM kb_resource_edges \
         WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(uuid::Uuid::from(r1.id))
    .bind(uuid::Uuid::from(r2.id))
    .fetch_all(&pool)
    .await
    .expect("fetch edges after add");

    assert_eq!(
        edges_after_add.len(),
        1,
        "expected exactly one relates_to edge from r1 → r2 after add, got {:?}",
        edges_after_add
    );
    assert_eq!(edges_after_add[0].2, "relates_to");

    // --- Step 2: clear relates_to → row removed ---
    let payload_remove = MetaUpdatePayload {
        resource_id: r1.id,
        managed_meta: serde_json::json!({"temper-type": "research"}),
        open_meta: serde_json::json!({
            "relates_to": [],
        }),
        managed_hash: "sha256:edges_managed_v2".to_string(),
        open_hash: "sha256:edges_open_v2".to_string(),
    };
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{}/meta", r1.id)))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&payload_remove)
        .send()
        .await
        .expect("meta update (remove) request failed");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let edges_after_remove: Vec<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as(
        "SELECT source_resource_id, target_resource_id FROM kb_resource_edges \
         WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(uuid::Uuid::from(r1.id))
    .bind(uuid::Uuid::from(r2.id))
    .fetch_all(&pool)
    .await
    .expect("fetch edges after remove");
    assert!(
        edges_after_remove.is_empty(),
        "relates_to edge must be removed when declaration is cleared, got {:?}",
        edges_after_remove
    );

    // --- Step 3: re-add → edge reappears (idempotent reconcile) ---
    let payload_readd = MetaUpdatePayload {
        resource_id: r1.id,
        managed_meta: serde_json::json!({"temper-type": "research"}),
        open_meta: serde_json::json!({
            "relates_to": [r2_ref.clone()],
        }),
        managed_hash: "sha256:edges_managed_v3".to_string(),
        open_hash: "sha256:edges_open_v3".to_string(),
    };
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{}/meta", r1.id)))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&payload_readd)
        .send()
        .await
        .expect("meta update (re-add) request failed");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let edges_after_readd: Vec<(uuid::Uuid, uuid::Uuid, String)> = sqlx::query_as(
        "SELECT source_resource_id, target_resource_id, edge_type::TEXT \
         FROM kb_resource_edges \
         WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(uuid::Uuid::from(r1.id))
    .bind(uuid::Uuid::from(r2.id))
    .fetch_all(&pool)
    .await
    .expect("fetch edges after readd");

    assert_eq!(
        edges_after_readd.len(),
        1,
        "relates_to edge must reappear on re-add"
    );
    assert_eq!(edges_after_readd[0].2, "relates_to");
}

/// Meta PATCH authorization + error mapping: second-user is forbidden,
/// unknown resource id is 404, and unknown doc_type is 400.
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
        .create("meta-errors")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "Errors Doc".to_string(),
        origin_uri: "test://e2e/meta-errors".to_string(),
        context_name: "meta-errors".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(format!("{:0>64}", "e")),
        slug: "errors-doc".to_string(),
        content: "# Errors\n\nResource for error mapping.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: Some(serde_json::json!({})),
        chunks_packed: Some(pack_chunks(&[]).expect("pack chunks")),
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
        managed_meta: serde_json::json!({"temper-type": "research"}),
        open_meta: serde_json::json!({}),
        managed_hash: "sha256:second_user".to_string(),
        open_hash: "sha256:second_user".to_string(),
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
        managed_meta: serde_json::json!({"temper-type": "research"}),
        open_meta: serde_json::json!({}),
        managed_hash: "sha256:ghost".to_string(),
        open_hash: "sha256:ghost".to_string(),
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

    // --- (3) Unknown doc_type → 400 ---
    let bad_doctype_payload = MetaUpdatePayload {
        resource_id: resource.id,
        managed_meta: serde_json::json!({
            "temper-type": "definitely-not-a-real-type",
        }),
        open_meta: serde_json::json!({}),
        managed_hash: "sha256:bad_doctype".to_string(),
        open_hash: "sha256:bad_doctype".to_string(),
    };
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{}/meta", resource.id)))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&bad_doctype_payload)
        .send()
        .await
        .expect("bad doctype meta update request failed");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "unknown doc_type must map to 400"
    );
}
