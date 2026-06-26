#![cfg(feature = "test-db")]

mod common;

use serde_json::json;
use temper_core::types::api::SearchParams;
use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

/// Helper: build an IngestPayload with a dummy embedding and optional open_meta.
fn test_payload(
    title: &str,
    slug: &str,
    context: &str,
    open_meta: Option<serde_json::Value>,
) -> IngestPayload {
    let dummy_embedding = vec![0.1_f32; 768];
    let chunks = vec![PackedChunk {
        chunk_index: 0,
        header_path: title.to_string(),
        heading_depth: 1,
        content: format!("{title} content for testing"),
        content_hash: format!("{slug}-hash"),
        embedding: dummy_embedding,
    }];

    IngestPayload {
        title: title.to_string(),
        origin_uri: format!("test://e2e/{slug}"),
        context_ref: format!("@me/{context}"),
        doc_type_name: "research".to_string(),
        content_hash: Some(
            format!("{slug}-body-hash-{pad}", pad = "0".repeat(64))[..64].to_string(),
        ),
        slug: slug.to_string(),
        content: format!("# {title}\n\n{title} content for testing."),
        metadata: None,
        managed_meta: Some(json!({"date": "2026-04-11"})),
        open_meta,
        chunks_packed: Some(pack_chunks(&chunks).expect("pack")),
    }
}

/// Ingest linked documents, verify graph expansion surfaces connected docs.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: frontmatter->edge auto-projection retired (depends_on not projected; temper-goal Edge-fate unprocessed in create_resource). Edges now via the relationship API; edge assert+read covered by temper-api relationship_handler_test. e2e edge-read + graph-expansion rewrite to the relationship API tracked (F7)"]
async fn graph_search_e2e_expands_connected_documents(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Ensure profile + context exist
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("graph-e2e")
        .await
        .expect("create context");

    // Ingest in dependency order: C (leaf) -> B (depends_on C) -> A (extends B)
    let payload_c = test_payload("Data Model", "data-model", "graph-e2e", None);
    let resource_c = app
        .client
        .ingest()
        .create(&payload_c)
        .await
        .expect("ingest C");

    let payload_b = test_payload(
        "Architecture Design",
        "architecture-design",
        "graph-e2e",
        Some(json!({"depends_on": ["data-model"]})),
    );
    let resource_b = app
        .client
        .ingest()
        .create(&payload_b)
        .await
        .expect("ingest B");

    let payload_a = test_payload(
        "Deployment Config",
        "deployment-config",
        "graph-e2e",
        Some(json!({"extends": ["architecture-design"]})),
    );
    let resource_a = app
        .client
        .ingest()
        .create(&payload_a)
        .await
        .expect("ingest A");

    // Search with graph_expand: true using Doc A as explicit seed
    let params_with_graph = SearchParams {
        context_name: Some("graph-e2e".into()),
        limit: Some(10),
        seed_ids: Some(vec![resource_a.id.into()]),
        graph_depth: Some(3),
        ..SearchParams::default()
    };

    let results = app
        .client
        .search()
        .search_with_params(&params_with_graph)
        .await
        .expect("graph search");

    let result_ids: Vec<uuid::Uuid> = results.iter().map(|r| r.resource_id).collect();

    assert!(
        result_ids.contains(&resource_b.id.into()),
        "Architecture Design should appear via graph (1 hop extends from A). Got: {result_ids:?}"
    );
    assert!(
        result_ids.contains(&resource_c.id.into()),
        "Data Model should appear via graph (2 hops extends->depends_on from A). Got: {result_ids:?}"
    );

    // Search with graph_expand: false -- seed_ids alone with no query/embedding
    // goes through unified_search which has no FTS/vector match for seeds
    let params_no_graph = SearchParams {
        graph_expand: false,
        ..params_with_graph.clone()
    };

    let results_no_graph = app
        .client
        .search()
        .search_with_params(&params_no_graph)
        .await
        .expect("non-graph search");

    let no_graph_ids: Vec<uuid::Uuid> = results_no_graph.iter().map(|r| r.resource_id).collect();

    // Without graph expansion, seed_ids alone won't produce graph-connected results
    assert!(
        !no_graph_ids.contains(&resource_b.id.into()),
        "Architecture Design should NOT appear without graph expansion"
    );
}

/// Verify the edges endpoint returns correct edges after ingest.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: frontmatter->edge auto-projection retired (depends_on not projected; temper-goal Edge-fate unprocessed in create_resource). Edges now via the relationship API; edge assert+read covered by temper-api relationship_handler_test. e2e edge-read + graph-expansion rewrite to the relationship API tracked (F7)"]
async fn edges_endpoint_returns_resource_edges(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("edges-e2e")
        .await
        .expect("create context");

    // Ingest B (leaf), then A (depends_on B)
    let payload_b = test_payload("Base Doc", "base-doc", "edges-e2e", None);
    let resource_b = app
        .client
        .ingest()
        .create(&payload_b)
        .await
        .expect("ingest B");

    let payload_a = test_payload(
        "Dependent Doc",
        "dependent-doc",
        "edges-e2e",
        Some(json!({"depends_on": ["base-doc"]})),
    );
    let resource_a = app
        .client
        .ingest()
        .create(&payload_a)
        .await
        .expect("ingest A");

    // Fetch edges for A
    let edges = app
        .client
        .resources()
        .edges(resource_a.id.into())
        .await
        .expect("fetch edges");

    assert_eq!(edges.len(), 1, "A should have 1 edge");
    assert_eq!(edges[0].label, "depends_on");
    assert_eq!(edges[0].direction, "outgoing");
    assert_eq!(edges[0].peer_slug, "base-doc");
    assert_eq!(edges[0].peer_resource_id, resource_b.id.0);

    // Fetch edges for B (should have incoming)
    let edges_b = app
        .client
        .resources()
        .edges(resource_b.id.into())
        .await
        .expect("fetch edges for B");
    assert_eq!(edges_b.len(), 1, "B should have 1 incoming edge");
    assert_eq!(edges_b[0].direction, "incoming");
    assert_eq!(edges_b[0].peer_slug, "dependent-doc");
}

/// Verify search_with_params respects graph flags end-to-end.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: frontmatter->edge auto-projection retired (depends_on not projected; temper-goal Edge-fate unprocessed in create_resource). Edges now via the relationship API; edge assert+read covered by temper-api relationship_handler_test. e2e edge-read + graph-expansion rewrite to the relationship API tracked (F7)"]
async fn search_no_graph_flag_disables_expansion(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("nograph-e2e")
        .await
        .expect("create context");

    let payload_b = test_payload("Leaf Node", "leaf-node", "nograph-e2e", None);
    let _resource_b = app
        .client
        .ingest()
        .create(&payload_b)
        .await
        .expect("ingest B");

    let payload_a = test_payload(
        "Root Node",
        "root-node",
        "nograph-e2e",
        Some(json!({"depends_on": ["leaf-node"]})),
    );
    let resource_a = app
        .client
        .ingest()
        .create(&payload_a)
        .await
        .expect("ingest A");

    // Search with explicit seed and graph enabled
    let params_graph = SearchParams {
        context_name: Some("nograph-e2e".into()),
        limit: Some(10),
        seed_ids: Some(vec![resource_a.id.into()]),
        graph_depth: Some(2),
        ..SearchParams::default()
    };

    let results_graph = app
        .client
        .search()
        .search_with_params(&params_graph)
        .await
        .expect("graph search");
    let graph_ids: Vec<uuid::Uuid> = results_graph.iter().map(|r| r.resource_id).collect();
    assert!(
        graph_ids.contains(&_resource_b.id.into()),
        "Leaf should appear via graph expansion. Got: {graph_ids:?}"
    );

    // Same but graph_expand: false
    let params_no_graph = SearchParams {
        graph_expand: false,
        ..params_graph.clone()
    };
    let results_no_graph = app
        .client
        .search()
        .search_with_params(&params_no_graph)
        .await
        .expect("no-graph search");
    let no_graph_ids: Vec<uuid::Uuid> = results_no_graph.iter().map(|r| r.resource_id).collect();
    assert!(
        !no_graph_ids.contains(&_resource_b.id.into()),
        "Leaf should NOT appear without graph expansion. Got: {no_graph_ids:?}"
    );
}
