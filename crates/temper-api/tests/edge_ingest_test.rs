#![cfg(feature = "test-db")]

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

// ─── Task 6: Edge Extraction on Ingest ─────────────────────────────────────

/// Extracting edges from open_meta with a resolvable slug creates an edge.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_extract_edges_from_open_meta(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "edge-ingest@test.com").await;

    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    let profile_id = ProfileId::from(profile);
    let context_id =
        ContextId::from(uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap());
    let resource_id = ResourceId::from(r1);

    let open_meta = json!({"extends": ["doc-b"]});
    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool,
        &profile_id,
        &context_id,
        &resource_id,
        "research",
        &serde_json::json!({}),
        &open_meta,
    )
    .await
    .expect("extract_and_upsert_edges");

    assert_eq!(created, 1, "should create 1 edge");
    assert_eq!(deferred, 0, "should defer 0 edges");

    // Verify the edge exists in the DB
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1 AND target_resource_id = $2 AND edge_type::TEXT = 'extends'",
    )
    .bind(r1)
    .bind(r2)
    .fetch_one(&pool)
    .await
    .expect("count edges");

    assert_eq!(count, 1, "edge r1->r2 should exist");
}

/// Unresolvable slug targets are stored as deferred edges.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_unresolved_targets_deferred(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "deferred@test.com").await;

    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;

    let profile_id = ProfileId::from(profile);
    let context_id =
        ContextId::from(uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap());
    let resource_id = ResourceId::from(r1);

    let open_meta = json!({"depends_on": ["nonexistent-slug"]});
    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool,
        &profile_id,
        &context_id,
        &resource_id,
        "research",
        &serde_json::json!({}),
        &open_meta,
    )
    .await
    .expect("extract_and_upsert_edges");

    assert_eq!(created, 0, "no edges should be created");
    assert_eq!(deferred, 1, "one edge should be deferred");

    // Verify deferred edge exists
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_deferred_edges WHERE source_resource_id = $1 AND target_ref = 'nonexistent-slug'",
    )
    .bind(r1)
    .fetch_one(&pool)
    .await
    .expect("count deferred");

    assert_eq!(count, 1, "deferred edge should exist");
}

/// Deferred edges are resolved when the target resource is created.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_deferred_edge_resolution(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "resolve@test.com").await;

    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;

    let profile_id = ProfileId::from(profile);
    let context_id =
        ContextId::from(uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap());
    let r1_id = ResourceId::from(r1);

    // Step 1: Create r1 with a forward reference to "future-doc"
    let open_meta = json!({"extends": ["future-doc"]});
    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool,
        &profile_id,
        &context_id,
        &r1_id,
        "research",
        &serde_json::json!({}),
        &open_meta,
    )
    .await
    .expect("extract_and_upsert_edges");

    assert_eq!(created, 0);
    assert_eq!(deferred, 1);

    // Step 2: Create r2 with slug "future-doc"
    let r2 =
        common::fixtures::create_test_resource(&pool, profile, "Future Doc", "future-doc").await;
    let r2_id = ResourceId::from(r2);

    // Step 3: Resolve deferred edges for r2
    let resolved = temper_api::services::edge_service::resolve_deferred_edges(
        &pool,
        &r2_id,
        Some("future-doc"),
        &profile_id,
    )
    .await
    .expect("resolve_deferred_edges");

    assert_eq!(resolved, 1, "should resolve 1 deferred edge");

    // Verify the real edge now exists
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1 AND target_resource_id = $2 AND edge_type::TEXT = 'extends'",
    )
    .bind(r1)
    .bind(r2)
    .fetch_one(&pool)
    .await
    .expect("count edges");

    assert_eq!(count, 1, "edge r1->r2 should exist after resolution");

    // Verify deferred table is empty
    let deferred_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_deferred_edges WHERE source_resource_id = $1")
            .bind(r1)
            .fetch_one(&pool)
            .await
            .expect("count deferred");

    assert_eq!(deferred_count, 0, "deferred table should be empty");
}

/// UUID string in open_meta resolves directly to the target resource.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_uuid_target_ref_resolves(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "uuid-ref@test.com").await;

    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    let profile_id = ProfileId::from(profile);
    let context_id =
        ContextId::from(uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap());
    let resource_id = ResourceId::from(r1);

    // Use r2's UUID string in the references field
    let open_meta = json!({"references": [r2.to_string()]});
    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool,
        &profile_id,
        &context_id,
        &resource_id,
        "research",
        &serde_json::json!({}),
        &open_meta,
    )
    .await
    .expect("extract_and_upsert_edges");

    assert_eq!(created, 1, "UUID reference should resolve to 1 edge");
    assert_eq!(deferred, 0, "no edges should be deferred");
}

/// open_meta with no relationship fields produces no edges.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_no_relationship_fields_no_edges(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "no-edges@test.com").await;

    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;

    let profile_id = ProfileId::from(profile);
    let context_id =
        ContextId::from(uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap());
    let resource_id = ResourceId::from(r1);

    let open_meta = json!({"some_custom_field": "value"});
    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool,
        &profile_id,
        &context_id,
        &resource_id,
        "research",
        &serde_json::json!({}),
        &open_meta,
    )
    .await
    .expect("extract_and_upsert_edges");

    assert_eq!(created, 0, "no edges should be created");
    assert_eq!(deferred, 0, "no edges should be deferred");
}

// ─── Task 7: Edge Reconciliation on Update ─────────────────────────────────

/// Reconciliation adds new edges and removes stale ones.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_reconcile_adds_and_removes(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "reconcile@test.com").await;

    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "Doc C", "doc-c").await;

    let profile_id = ProfileId::from(profile);
    let context_id =
        ContextId::from(uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap());
    let resource_id = ResourceId::from(r1);

    // Initial: r1 extends r2
    let open_meta_v1 = json!({"extends": ["doc-b"]});
    temper_api::services::edge_service::extract_and_upsert_edges(
        &pool,
        &profile_id,
        &context_id,
        &resource_id,
        "research",
        &serde_json::json!({}),
        &open_meta_v1,
    )
    .await
    .expect("initial extract");

    // Reconcile: r1 now extends r3 instead of r2
    let open_meta_v2 = json!({"extends": ["doc-c"]});
    let result = temper_api::services::edge_service::reconcile_edges(
        &pool,
        &profile_id,
        &context_id,
        &resource_id,
        "research",
        &serde_json::json!({}),
        &open_meta_v2,
    )
    .await
    .expect("reconcile_edges");

    assert_eq!(result.added, 1, "should add r1->r3");
    assert_eq!(result.removed, 1, "should remove r1->r2");

    // Verify r1->r2 gone
    let old_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(r1)
    .bind(r2)
    .fetch_one(&pool)
    .await
    .expect("count old edge");
    assert_eq!(old_count, 0, "r1->r2 should be removed");

    // Verify r1->r3 exists
    let new_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(r1)
    .bind(r3)
    .fetch_one(&pool)
    .await
    .expect("count new edge");
    assert_eq!(new_count, 1, "r1->r3 should exist");
}

/// Reconciliation preserves manually-created edges (non-frontmatter provenance).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_reconcile_preserves_manual_edges(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "manual@test.com").await;

    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    let profile_id = ProfileId::from(profile);
    let context_id =
        ContextId::from(uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap());
    let resource_id = ResourceId::from(r1);

    // Manually insert an edge with manual provenance
    let manual_id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_resource_edges
            (id, source_resource_id, target_resource_id, edge_type, weight, metadata, created_by_profile_id)
           VALUES ($1, $2, $3, 'extends'::edge_type, 1.0, '{"provenance": "manual"}', $4)"#,
    )
    .bind(manual_id)
    .bind(r1)
    .bind(r2)
    .bind(profile)
    .execute(&pool)
    .await
    .expect("insert manual edge");

    // Reconcile with empty open_meta — should NOT touch the manual edge
    let open_meta = json!({});
    let result = temper_api::services::edge_service::reconcile_edges(
        &pool,
        &profile_id,
        &context_id,
        &resource_id,
        "research",
        &serde_json::json!({}),
        &open_meta,
    )
    .await
    .expect("reconcile_edges");

    assert_eq!(result.removed, 0, "manual edge should not be removed");

    // Verify manual edge still exists
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_resource_edges WHERE id = $1")
        .bind(manual_id)
        .fetch_one(&pool)
        .await
        .expect("count manual edge");
    assert_eq!(count, 1, "manual edge should still exist");
}

/// ParentOf edges reverse direction: child declares parent, edge is parent->child.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_parent_of_direction_reversal(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "parent@test.com").await;

    let child =
        common::fixtures::create_test_resource(&pool, profile, "Child Doc", "child-doc").await;
    let parent =
        common::fixtures::create_test_resource(&pool, profile, "Parent Doc", "parent-doc").await;

    let profile_id = ProfileId::from(profile);
    let context_id =
        ContextId::from(uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap());
    let child_id = ResourceId::from(child);

    // Child declares its parent
    let open_meta = json!({"parent": "parent-doc"});
    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool,
        &profile_id,
        &context_id,
        &child_id,
        "research",
        &serde_json::json!({}),
        &open_meta,
    )
    .await
    .expect("extract_and_upsert_edges");

    assert_eq!(created, 1, "should create 1 edge");
    assert_eq!(deferred, 0, "should defer 0 edges");

    // Verify the edge is parent->child (source=parent, target=child) with edge_type='parent_of'
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1 AND target_resource_id = $2 AND edge_type::TEXT = 'parent_of'",
    )
    .bind(parent)
    .bind(child)
    .fetch_one(&pool)
    .await
    .expect("count parent_of edge");

    assert_eq!(count, 1, "edge should be parent->child with type parent_of");
}
