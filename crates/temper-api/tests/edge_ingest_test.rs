#![cfg(feature = "test-db")]

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

// ─── Task 6: Edge Extraction on Ingest ─────────────────────────────────────

/// Extracting edges from open_meta with a resolvable slug creates an edge and
/// a corresponding `relationship_asserted` ledger event.
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
    let (projected, pending) = temper_api::services::edge_service::extract_and_upsert_edges(
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

    assert_eq!(projected, 1, "should project 1 edge");
    assert_eq!(pending, 0, "should leave 0 pending");

    // Verify the projected edge row exists (label == legacy frontmatter name).
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1
           AND target_resource_id = $2 AND label = 'extends' AND NOT is_folded",
    )
    .bind(r1)
    .bind(r2)
    .fetch_one(&pool)
    .await
    .expect("count edges");

    assert_eq!(count, 1, "edge r1->r2 should exist");

    // Verify a corresponding `relationship_asserted` event landed.
    let event_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM kb_events ev
            JOIN kb_event_types et ON et.id = ev.event_type_id
           WHERE et.name = 'relationship_asserted'
             AND (ev.payload->>'source_resource_id')::uuid = $1
             AND (ev.payload->'target'->>'value')::uuid = $2
             AND ev.metadata->>'intent' = 'derived'"#,
    )
    .bind(r1)
    .bind(r2)
    .fetch_one(&pool)
    .await
    .expect("count events");
    assert_eq!(event_count, 1, "one relationship_asserted event expected");
}

/// Unresolvable slug targets become pending `relationship_asserted` events
/// (slug TargetEndpoint) rather than projected rows.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_unresolved_targets_become_pending_assertions(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "deferred@test.com").await;

    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;

    let profile_id = ProfileId::from(profile);
    let context_id =
        ContextId::from(uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap());
    let resource_id = ResourceId::from(r1);

    let open_meta = json!({"depends_on": ["nonexistent-slug"]});
    let (projected, pending) = temper_api::services::edge_service::extract_and_upsert_edges(
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

    assert_eq!(projected, 0, "no edges should be projected");
    assert_eq!(pending, 1, "one slug-target assertion expected");

    // Verify the pending event landed with a slug TargetEndpoint.
    let event_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM kb_events ev
            JOIN kb_event_types et ON et.id = ev.event_type_id
           WHERE et.name = 'relationship_asserted'
             AND (ev.payload->>'source_resource_id')::uuid = $1
             AND ev.payload->'target'->>'kind' = 'slug'
             AND ev.payload->'target'->>'value' = 'nonexistent-slug'
             AND ev.metadata->>'intent' = 'derived'"#,
    )
    .bind(r1)
    .fetch_one(&pool)
    .await
    .expect("count slug assertion events");
    assert_eq!(event_count, 1, "one slug-target assertion event expected");
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

    let open_meta = json!({"references": [r2.to_string()]});
    let (projected, pending) = temper_api::services::edge_service::extract_and_upsert_edges(
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

    assert_eq!(projected, 1, "UUID reference should project 1 edge");
    assert_eq!(pending, 0, "no pending assertions");
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
    let (projected, pending) = temper_api::services::edge_service::extract_and_upsert_edges(
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

    assert_eq!(projected, 0, "no edges should be projected");
    assert_eq!(pending, 0, "no pending assertions");
}

// ─── Task 7: Edge Reconciliation on Update ─────────────────────────────────

/// Reconciliation adds new edges and folds stale ones, emitting matching
/// `relationship_asserted` / `relationship_folded` events.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_reconcile_adds_and_folds(pool: PgPool) {
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
    assert_eq!(result.removed, 1, "should fold r1->r2");

    // r1->r2 is folded, not deleted.
    let folded_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1
            AND target_resource_id = $2 AND is_folded = true",
    )
    .bind(r1)
    .bind(r2)
    .fetch_one(&pool)
    .await
    .expect("count folded edge");
    assert_eq!(folded_count, 1, "r1->r2 should be folded");

    // r1->r3 active.
    let active_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1
            AND target_resource_id = $2 AND NOT is_folded",
    )
    .bind(r1)
    .bind(r3)
    .fetch_one(&pool)
    .await
    .expect("count new edge");
    assert_eq!(active_count, 1, "r1->r3 should exist and be active");

    // A `relationship_folded` event was appended, correlated with the
    // original `relationship_asserted` for r1->r2.
    let folded_event_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM kb_events ev
            JOIN kb_event_types et ON et.id = ev.event_type_id
           WHERE et.name = 'relationship_folded'
             AND ev.metadata->>'intent' = 'derived'"#,
    )
    .fetch_one(&pool)
    .await
    .expect("count fold events");
    assert_eq!(
        folded_event_count, 1,
        "one relationship_folded event expected"
    );
}

/// Reconciliation preserves manually-asserted edges (non-frontmatter event source).
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

    // Synthesize a non-frontmatter assertion event + edge row (e.g. a manual
    // CLI assert from a future surface). The fixture stamps source='fixture',
    // which the reconcile-time JOIN excludes (filter is source='frontmatter').
    let manual_edge = common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;

    // Reconcile with empty open_meta — should NOT touch the manual edge.
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

    assert_eq!(result.removed, 0, "manual edge should not be folded");

    // Verify the manual edge still exists and is not folded.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE id = $1 AND NOT is_folded",
    )
    .bind(manual_edge)
    .fetch_one(&pool)
    .await
    .expect("count manual edge");
    assert_eq!(count, 1, "manual edge should still exist and be active");
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
    let (projected, pending) = temper_api::services::edge_service::extract_and_upsert_edges(
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

    assert_eq!(projected, 1, "should project 1 edge");
    assert_eq!(pending, 0, "should leave 0 pending");

    // Verify the edge is parent->child with label='parent_of'.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1
            AND target_resource_id = $2 AND label = 'parent_of' AND NOT is_folded",
    )
    .bind(parent)
    .bind(child)
    .fetch_one(&pool)
    .await
    .expect("count parent_of edge");

    assert_eq!(
        count, 1,
        "edge should be parent->child with label parent_of"
    );
}
