#![cfg(feature = "test-db")]

mod common;

use sqlx::PgPool;
use temper_core::types::api::UnifiedSearchResultRow;

/// graph_search returns graph-connected resources by expanding from seed IDs.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_graph_search_expands_from_seeds(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "gsearch@test.com").await;

    // Create three resources: A extends B, B depends_on C
    let a = common::fixtures::create_test_resource(&pool, profile, "Doc Alpha", "doc-alpha").await;
    let b = common::fixtures::create_test_resource(&pool, profile, "Doc Beta", "doc-beta").await;
    let c = common::fixtures::create_test_resource(&pool, profile, "Doc Gamma", "doc-gamma").await;

    // Create edges: A→B (extends), B→C (depends_on)
    common::fixtures::create_test_edge(&pool, a, b, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, b, c, "depends_on", profile).await;

    // Call graph_search with explicit seed = A, no query/embedding
    let results: Vec<UnifiedSearchResultRow> = sqlx::query_as(
        r#"
        SELECT resource_id, title, slug, kb_uri, origin_uri,
               context, doc_type, fts_score, vector_score,
               combined_score, origin
          FROM graph_search($1, '', NULL, 'english', NULL, NULL, 0.5, 0.5,
                           $2, '{}', 2, 0.3, 10, 0)
        "#,
    )
    .bind(profile)
    .bind(vec![a])
    .fetch_all(&pool)
    .await
    .expect("graph_search query");

    let result_ids: Vec<uuid::Uuid> = results.iter().map(|r| r.resource_id).collect();

    // B should appear via graph (1 hop from A via extends)
    assert!(
        result_ids.contains(&b),
        "Doc Beta should appear via graph expansion (1 hop). Got: {result_ids:?}"
    );
    // C should appear via graph (2 hops from A)
    assert!(
        result_ids.contains(&c),
        "Doc Gamma should appear via graph expansion (2 hops). Got: {result_ids:?}"
    );

    // Graph-only results should have origin = 'graph'
    let beta_result = results.iter().find(|r| r.resource_id == b).unwrap();
    assert_eq!(beta_result.origin, "graph");
}

/// graph_search with no edges degrades to unified_search results.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_graph_search_no_edges_degrades(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "degrade@test.com").await;
    let a = common::fixtures::create_test_resource(&pool, profile, "Isolated Doc", "isolated-doc")
        .await;

    // Insert a chunk so vector search can find it.
    // The 4-arg persist_resource_chunks requires an audit row; seed one first.
    let event_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events \
         (id, profile_id, device_id, kb_context_id, resource_id, event_type_id, payload, created) \
         VALUES (gen_random_uuid(), $1, 'test-device', \
             (SELECT kb_context_id FROM kb_resources WHERE id = $2), \
             $2, (SELECT id FROM kb_event_types WHERE name = 'resource_created'), '{}', now()) RETURNING id",
    )
    .bind(profile)
    .bind(a)
    .fetch_one(&pool)
    .await
    .expect("seed event");

    let audit_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resource_audits \
         (resource_id, event_id, profile_id, device_id, body_hash, managed_hash, open_hash, action) \
         VALUES ($1, $2, $3, 'test-device', 'iso-body', 'mh', 'oh', 'create') RETURNING id",
    )
    .bind(a)
    .bind(event_id)
    .bind(profile)
    .fetch_one(&pool)
    .await
    .expect("seed audit");

    let chunk_json = serde_json::json!([{
        "chunk_index": 0,
        "header_path": "Isolated",
        "heading_depth": 1,
        "content": "Isolated content",
        "content_hash": "iso-hash",
        "embedding": format!("[{}]", vec!["0.1"; 768].join(","))
    }]);
    sqlx::query("SELECT persist_resource_chunks($1::uuid, $2::uuid, $3::text, $4::jsonb)")
        .bind(a)
        .bind(audit_id)
        .bind("iso-body")
        .bind(&chunk_json)
        .execute(&pool)
        .await
        .expect("persist chunks");

    // Search with a matching embedding
    let embedding_str = format!("[{}]", vec!["0.1"; 768].join(","));
    let results: Vec<UnifiedSearchResultRow> = sqlx::query_as(
        r#"
        SELECT resource_id, title, slug, kb_uri, origin_uri,
               context, doc_type, fts_score, vector_score,
               combined_score, origin
          FROM graph_search($1, '', $2::vector, 'english', NULL, NULL, 0.0, 1.0,
                           '{}', '{}', 2, 0.3, 10, 0)
        "#,
    )
    .bind(profile)
    .bind(&embedding_str)
    .fetch_all(&pool)
    .await
    .expect("graph_search with no edges");

    assert!(
        !results.is_empty(),
        "should find the isolated doc via vector search"
    );
    assert!(
        results.iter().any(|r| r.resource_id == a),
        "isolated doc should appear in results"
    );
}
