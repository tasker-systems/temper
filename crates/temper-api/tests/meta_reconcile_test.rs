#![cfg(feature = "test-db")]

//! C1: meta-only updates must reconcile frontmatter-provenance edges.
//!
//! The meta-only sync path (`meta_service::update_meta`) updates
//! `kb_resource_manifests.open_meta` without re-chunking. Before C1 it did
//! not call `edge_service::reconcile_edges`, so relationship frontmatter
//! written by `temper graph build` never produced knowledge-graph edges
//! on the server. These tests pin the fixed behavior.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_core::types::ids::ResourceId;
use temper_core::types::managed_meta::{ManagedMeta, MetaUpdatePayload};

/// Build a MetaUpdatePayload with the given open_meta value. Uses stable
/// dummy hashes — the service just stores them, it does not validate.
fn meta_payload(resource_id: uuid::Uuid, open_meta: serde_json::Value) -> MetaUpdatePayload {
    MetaUpdatePayload {
        resource_id: ResourceId::from(resource_id),
        managed_meta: ManagedMeta::default(),
        open_meta,
        managed_hash: "test-mhash".to_string(),
        open_hash: "test-ohash".to_string(),
    }
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
    let payload = meta_payload(source, json!({"extends": ["target-doc"]}));
    temper_api::services::meta_service::update_meta(
        &pool,
        profile_id,
        ResourceId::from(source),
        "test-device",
        payload,
    )
    .await
    .expect("update_meta with extends");

    // The edge should exist: source -> target, type extends, frontmatter provenance.
    let created_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM kb_resource_edges
           WHERE source_resource_id = $1
             AND target_resource_id = $2
             AND edge_type::TEXT = 'extends'
             AND metadata->>'provenance' = 'frontmatter'"#,
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

    // --- Act 2: meta update removes the relationship (empty open_meta) ---
    let payload = meta_payload(source, json!({}));
    temper_api::services::meta_service::update_meta(
        &pool,
        profile_id,
        ResourceId::from(source),
        "test-device",
        payload,
    )
    .await
    .expect("update_meta clearing relationships");

    // The frontmatter edge must be gone.
    let remaining: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM kb_resource_edges
           WHERE source_resource_id = $1
             AND target_resource_id = $2"#,
    )
    .bind(source)
    .bind(target)
    .fetch_one(&pool)
    .await
    .expect("count edges after clear");
    assert_eq!(
        remaining, 0,
        "clearing extends via meta update must reconcile the edge away"
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
