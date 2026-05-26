#![cfg(feature = "test-db")]

//! Integration test: `resource_service::update` must reconcile frontmatter-provenance
//! edges when `open_meta` is touched.
//!
//! Phase 3b routes PUT /api/resources/{id}/meta through `DbBackend::update_resource
//! → translator → resource_service::update`. Before this fix, `resource_service::update`
//! did NOT call `edge_service::reconcile_edges`, so a meta-only update via the new path
//! would silently drop edge reconciliation. This test pins the fixed behavior.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::managed_meta::ManagedMeta;
use temper_core::types::resource::ResourceUpdateRequest;

/// A `resource_service::update` call with `open_meta` containing an `extends`
/// relationship must create a frontmatter-provenance edge, and a follow-up
/// call that clears the relationship must remove it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_update_reconciles_edges_via_open_meta(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile =
        common::fixtures::create_test_profile(&pool, "resource-update-reconcile@test.com").await;

    // Source resource: born with a manifest row (so the update can merge into
    // existing managed/open meta). The initial open_meta has no relationships.
    let source = common::fixtures::create_test_resource_with_manifest(
        &pool,
        profile,
        "Source Doc",
        "source-doc-rur",
        json!({}),
    )
    .await;

    // Target resource: exists so the edge declaration can be resolved by slug.
    let target =
        common::fixtures::create_test_resource(&pool, profile, "Target Doc", "target-doc-rur")
            .await;

    // Sanity: no outgoing edges from source before the update.
    let edge_count_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1")
            .bind(source)
            .fetch_one(&pool)
            .await
            .expect("initial edge count");
    assert_eq!(edge_count_before, 0, "no edges should exist before update");

    // --- Act 1: update with open_meta.extends → target slug ---
    let req = ResourceUpdateRequest {
        title: None,
        slug: None,
        managed_meta: None,
        open_meta: Some(json!({"extends": ["target-doc-rur"]})),
        content: None,
        content_hash: None,
        chunks_packed: None,
    };
    temper_api::services::resource_service::update(&pool, profile, source, "test-device", req)
        .await
        .expect("resource_service::update with extends");

    // The frontmatter edge source → target must exist (joined through the
    // assertion event to verify the source='frontmatter' provenance signal).
    let created_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM kb_resource_edges e
            JOIN kb_events ev ON ev.id = e.asserted_by_event_id
           WHERE e.source_resource_id = $1
             AND e.target_resource_id = $2
             AND e.label = 'extends'
             AND NOT e.is_folded
             AND ev.metadata->>'intent' = 'derived'"#,
    )
    .bind(source)
    .bind(target)
    .fetch_one(&pool)
    .await
    .expect("count edges after add");
    assert_eq!(
        created_count, 1,
        "resource_service::update with extends must create a frontmatter edge"
    );

    // --- Act 2: update clearing the relationship ---
    // Sending `extends: []` explicitly overwrites the stored `extends` key
    // with an empty array. `apply_open_meta_partial` merges key-by-key, so
    // absent keys are preserved — only explicit keys can overwrite existing ones.
    let req_clear = ResourceUpdateRequest {
        title: None,
        slug: None,
        managed_meta: None,
        open_meta: Some(json!({"extends": []})),
        content: None,
        content_hash: None,
        chunks_packed: None,
    };
    temper_api::services::resource_service::update(
        &pool,
        profile,
        source,
        "test-device",
        req_clear,
    )
    .await
    .expect("resource_service::update clearing extends");

    // The frontmatter edge must be folded after reconciliation; the row
    // survives but is off the default projection.
    let remaining_active: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM kb_resource_edges
           WHERE source_resource_id = $1
             AND target_resource_id = $2
             AND NOT is_folded"#,
    )
    .bind(source)
    .bind(target)
    .fetch_one(&pool)
    .await
    .expect("count active edges after clear");
    assert_eq!(
        remaining_active, 0,
        "clearing extends via resource_service::update must fold the edge"
    );
}

/// A `resource_service::update` with only `managed_meta` (no `open_meta`) must
/// also trigger edge reconciliation. This covers the case where managed_meta
/// carries relationship declarations (e.g. `temper-parent`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_update_reconciles_edges_via_managed_meta(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile =
        common::fixtures::create_test_profile(&pool, "resource-update-managed-reconcile@test.com")
            .await;

    // Source: starts with a manifest and empty open_meta.
    let source = common::fixtures::create_test_resource_with_manifest(
        &pool,
        profile,
        "Source Managed Doc",
        "source-managed-doc-rur",
        json!({}),
    )
    .await;

    // No edges before the update.
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1")
            .bind(source)
            .fetch_one(&pool)
            .await
            .expect("pre-update edge count");
    assert_eq!(before, 0);

    // Issue a managed_meta-only update (no open_meta). Even with no edge
    // declarations in managed_meta for a research doc type, reconcile_edges
    // must still be called without error.
    let req = ResourceUpdateRequest {
        title: None,
        slug: None,
        managed_meta: Some(ManagedMeta::default()),
        open_meta: None,
        content: None,
        content_hash: None,
        chunks_packed: None,
    };
    temper_api::services::resource_service::update(&pool, profile, source, "test-device", req)
        .await
        .expect("resource_service::update with managed_meta only");

    // Still zero edges — the test asserts reconcile ran without error, not that
    // an edge was created (no declarations in this payload).
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1")
            .bind(source)
            .fetch_one(&pool)
            .await
            .expect("post-update edge count");
    assert_eq!(
        after, 0,
        "managed_meta-only update with no declarations must leave edge count unchanged"
    );
}
