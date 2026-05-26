#![cfg(feature = "test-db")]

//! C1: meta-only updates must reconcile frontmatter-provenance edges.
//!
//! Before C1 the meta-only update path did not call
//! `edge_service::reconcile_edges`, so relationship frontmatter from
//! sync-driven meta updates never produced knowledge-graph edges on the
//! server. These tests pin the fixed behavior. Dispatch is now through
//! `DbBackend::update_resource` (translator's meta-only branch).

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_api::backend::DbBackend;
use temper_core::operations::{Backend, ResourceRef, Surface, UpdateResource};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::managed_meta::ManagedMeta;

/// Dispatch a meta-only update with the given open_meta. Mirrors what the
/// MCP `update_resource_meta` tool does: build an UpdateResource cmd with
/// `body=None`, dispatch through DbBackend.
async fn meta_only_update(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: uuid::Uuid,
    open_meta: serde_json::Value,
) {
    let cmd = UpdateResource {
        resource: ResourceRef::Uuid {
            id: ResourceId::from(resource_id),
        },
        body: None,
        managed_meta: Some(ManagedMeta::default()),
        open_meta: Some(open_meta),
        move_to: None,
        origin: Surface::Mcp,
    };
    DbBackend::new(
        pool.clone(),
        profile_id,
        "test-device".to_string(),
        Surface::Mcp,
    )
    .update_resource(cmd)
    .await
    .expect("meta-only update via DbBackend");
}

/// A meta-only update with a new `extends` relationship must create the
/// frontmatter-provenance edge, and a follow-up update that clears the
/// relationship must remove it. Chunk state must be untouched throughout.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn meta_update_reconciles_edges(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "meta-reconcile@test.com").await;
    let profile_id = ProfileId::from(profile);

    // Source resource: start with an empty manifest, no relationships.
    let source = common::fixtures::create_test_resource_with_manifest(
        &pool,
        profile,
        "Source Doc",
        "source-doc",
        json!({}),
    )
    .await;
    // Target resource: exists so edge declarations can resolve by slug.
    let target =
        common::fixtures::create_test_resource(&pool, profile, "Target Doc", "target-doc").await;

    // Sanity: no outgoing edges from source yet.
    let edge_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1")
            .bind(source)
            .fetch_one(&pool)
            .await
            .expect("initial edge count");
    assert_eq!(edge_count, 0, "no edges should exist before update_meta");

    // Snapshot chunk state so we can assert meta update does not touch chunks.
    let chunks_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_chunks WHERE resource_id = $1")
            .bind(source)
            .fetch_one(&pool)
            .await
            .expect("count chunks before");

    // --- Act 1: meta update adds an `extends` relationship ---
    meta_only_update(
        &pool,
        profile_id,
        source,
        json!({"extends": ["target-doc"]}),
    )
    .await;

    // The edge should exist: source -> target, label 'extends', via a
    // frontmatter-sourced relationship_asserted event.
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
        "update_meta with extends must create a frontmatter edge"
    );

    // Chunks must be untouched by the meta update.
    let chunks_after_add: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_chunks WHERE resource_id = $1")
            .bind(source)
            .fetch_one(&pool)
            .await
            .expect("count chunks after add");
    assert_eq!(
        chunks_after_add, chunks_before,
        "meta update must not change chunk rows"
    );

    // --- Act 2: meta update clears the relationship by setting extends=[] ---
    // Partial-merge semantics: passing `{}` would be a no-op; the caller must
    // explicitly set `extends: []` to retract the declaration. Mirrors the
    // pattern in tests/e2e/tests/meta_test.rs::meta_patch_reconciles_edges_add_and_remove.
    meta_only_update(&pool, profile_id, source, json!({"extends": []})).await;

    // The frontmatter edge must be folded (off the default projection).
    // Rows survive in the table; the default projection filters them out
    // via `NOT is_folded`.
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
        "clearing extends via meta update must fold the edge"
    );

    // Chunk state still untouched.
    let chunks_after_clear: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_chunks WHERE resource_id = $1")
            .bind(source)
            .fetch_one(&pool)
            .await
            .expect("count chunks after clear");
    assert_eq!(
        chunks_after_clear, chunks_before,
        "second meta update must not change chunk rows"
    );
}
