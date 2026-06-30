#![cfg(feature = "test-db")]
//! E2e coverage for Surface-A graph search through the real `/api/search` stack.
//!
//! These tests close the thin-Rust-surface gap above the SQL engine: the
//! substrate `artifact-tests` exercise `search_graph_expand` / `unified_search`
//! directly, but nothing drove `search_select` (substrate_read.rs) end to end
//! with live edges. They previously relied on a frontmatter→edge
//! auto-projection at ingest (`open_meta` `depends_on`/`extends` arrays) that
//! was retired; edges now come from the relationship API
//! (`POST /api/relationships`). Each test creates resources via ingest, asserts
//! the edges explicitly, then searches.

mod common;

use serde_json::json;
use temper_core::types::api::SearchParams;
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::ResourceId;
use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use temper_core::types::relationship_requests::AssertRelationshipRequest;

/// Helper: build an IngestPayload with a dummy embedding.
fn test_payload(title: &str, slug: &str, context: &str) -> IngestPayload {
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
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(
            format!("{slug}-body-hash-{pad}", pad = "0".repeat(64))[..64].to_string(),
        ),
        slug: slug.to_string(),
        content: format!("# {title}\n\n{title} content for testing."),
        metadata: None,
        managed_meta: Some(json!({"date": "2026-04-11"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&chunks).expect("pack")),
        act: Default::default(),
    }
}

/// Assert a directed edge `source → target` via the relationship API.
///
/// `edge_kind`/`polarity` are immaterial to graph *expansion* — traversal is
/// symmetric and follows every kind when `edge_types` is empty (Beat 2 spec
/// §3.2) — so we use `LeadsTo`/`Forward` uniformly. `label` is the human-facing
/// relation name the edges endpoint surfaces.
async fn assert_edge(
    app: &common::E2eTestApp,
    source: ResourceId,
    target: ResourceId,
    label: &str,
) {
    app.client
        .relationships()
        .assert(&AssertRelationshipRequest {
            source,
            target,
            edge_kind: EdgeKind::LeadsTo,
            polarity: Polarity::Forward,
            label: label.to_string(),
            weight: 1.0,
            act: Default::default(),
        })
        .await
        .unwrap_or_else(|e| panic!("assert edge {label}: {e:?}"));
}

/// Graph expansion surfaces structurally-connected docs with a non-zero
/// `graph_score`, and `graph_expand: false` suppresses them.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn graph_search_e2e_expands_connected_documents(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("graph-e2e", None)
        .await
        .expect("create context");

    // Ingest a 3-node chain, then wire it: A --extends--> B --depends_on--> C.
    let resource_c = app
        .client
        .ingest()
        .create(&test_payload("Data Model", "data-model", "graph-e2e"))
        .await
        .expect("ingest C");
    let resource_b = app
        .client
        .ingest()
        .create(&test_payload(
            "Architecture Design",
            "architecture-design",
            "graph-e2e",
        ))
        .await
        .expect("ingest B");
    let resource_a = app
        .client
        .ingest()
        .create(&test_payload(
            "Deployment Config",
            "deployment-config",
            "graph-e2e",
        ))
        .await
        .expect("ingest A");

    assert_edge(&app, resource_a.id, resource_b.id, "extends").await;
    assert_edge(&app, resource_b.id, resource_c.id, "depends_on").await;

    // Search with Doc A as an explicit seed; graph expansion on.
    let params_with_graph = SearchParams {
        context_ref: Some("@me/graph-e2e".into()),
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

    let b_hit = results
        .iter()
        .find(|r| r.resource_id == resource_b.id.0)
        .unwrap_or_else(|| {
            panic!(
                "Architecture Design should surface via graph (1 hop from A). Got: {:?}",
                results.iter().map(|r| r.resource_id).collect::<Vec<_>>()
            )
        });
    assert!(
        b_hit.graph_score > 0.0,
        "1-hop neighbor must carry a non-zero graph_score; got {}",
        b_hit.graph_score
    );

    let c_hit = results
        .iter()
        .find(|r| r.resource_id == resource_c.id.0)
        .unwrap_or_else(|| {
            panic!(
                "Data Model should surface via graph (2 hops from A). Got: {:?}",
                results.iter().map(|r| r.resource_id).collect::<Vec<_>>()
            )
        });
    assert!(
        c_hit.graph_score > 0.0,
        "2-hop neighbor must carry a non-zero graph_score; got {}",
        c_hit.graph_score
    );

    // With graph_expand: false, seed_ids alone (no query/embedding) produce no
    // FTS/vector match, so the structural neighbors drop out entirely.
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
    assert!(
        !no_graph_ids.contains(&resource_b.id.into()),
        "Architecture Design should NOT appear without graph expansion"
    );
    assert!(
        !no_graph_ids.contains(&resource_c.id.into()),
        "Data Model should NOT appear without graph expansion"
    );
}

/// The `/api/resources/{id}/edges` endpoint returns the edges asserted via the
/// relationship API, with correct direction and peer fields on each end.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn edges_endpoint_returns_resource_edges(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("edges-e2e", None)
        .await
        .expect("create context");

    let resource_b = app
        .client
        .ingest()
        .create(&test_payload("Base Doc", "base-doc", "edges-e2e"))
        .await
        .expect("ingest B");
    let resource_a = app
        .client
        .ingest()
        .create(&test_payload("Dependent Doc", "dependent-doc", "edges-e2e"))
        .await
        .expect("ingest A");

    // A --depends_on--> B
    assert_edge(&app, resource_a.id, resource_b.id, "depends_on").await;

    // A's view: one outgoing edge to B.
    let edges_a = app
        .client
        .resources()
        .edges(resource_a.id.into())
        .await
        .expect("fetch edges for A");
    assert_eq!(edges_a.len(), 1, "A should have 1 edge");
    assert_eq!(edges_a[0].label, "depends_on");
    assert_eq!(edges_a[0].direction, "outgoing");
    assert_eq!(edges_a[0].peer_slug, "base-doc");
    assert_eq!(edges_a[0].peer_resource_id, resource_b.id);

    // B's view: the same edge, incoming.
    let edges_b = app
        .client
        .resources()
        .edges(resource_b.id.into())
        .await
        .expect("fetch edges for B");
    assert_eq!(edges_b.len(), 1, "B should have 1 incoming edge");
    assert_eq!(edges_b[0].direction, "incoming");
    assert_eq!(edges_b[0].peer_slug, "dependent-doc");
    assert_eq!(edges_b[0].peer_resource_id, resource_a.id);
}

/// `graph_expand` toggles expansion end to end: on ⇒ the neighbor surfaces,
/// off ⇒ it does not.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn search_no_graph_flag_disables_expansion(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("nograph-e2e", None)
        .await
        .expect("create context");

    let resource_b = app
        .client
        .ingest()
        .create(&test_payload("Leaf Node", "leaf-node", "nograph-e2e"))
        .await
        .expect("ingest B");
    let resource_a = app
        .client
        .ingest()
        .create(&test_payload("Root Node", "root-node", "nograph-e2e"))
        .await
        .expect("ingest A");

    assert_edge(&app, resource_a.id, resource_b.id, "depends_on").await;

    let params_graph = SearchParams {
        context_ref: Some("@me/nograph-e2e".into()),
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
        graph_ids.contains(&resource_b.id.into()),
        "Leaf should appear via graph expansion. Got: {graph_ids:?}"
    );

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
        !no_graph_ids.contains(&resource_b.id.into()),
        "Leaf should NOT appear without graph expansion. Got: {no_graph_ids:?}"
    );
}

/// `context_ref` scopes the candidate corpus — including graph-reached neighbors
/// — and an unresolvable ref errors rather than silently widening.
///
/// This replaces the original "unknown context returns empty" criterion: the
/// shipped `search_select` (substrate_read.rs:330) resolves `context_ref`
/// strictly, so an unknown context yields an error, not an empty result. The
/// load-bearing guarantee — that the filter is actually applied to graph hits
/// (the `corpus` CTE filters `blend ∪ graph`, migration 20260626000002) — is
/// asserted directly: a graph-reachable neighbor in another context is excluded.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn search_context_ref_scopes_and_unknown_errors(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("scope-a", None)
        .await
        .expect("create context scope-a");
    app.client
        .contexts()
        .create("scope-b", None)
        .await
        .expect("create context scope-b");

    let resource_a = app
        .client
        .ingest()
        .create(&test_payload("Anchor Doc", "anchor-doc", "scope-a"))
        .await
        .expect("ingest A in scope-a");
    let resource_d = app
        .client
        .ingest()
        .create(&test_payload("Foreign Doc", "foreign-doc", "scope-b"))
        .await
        .expect("ingest D in scope-b");

    // A (scope-a) --relates_to--> D (scope-b): a real, graph-reachable edge that
    // crosses the context boundary.
    assert_edge(&app, resource_a.id, resource_d.id, "relates_to").await;

    // Unscoped search from A reaches D — proves the edge exists and would surface
    // absent a context filter.
    let unscoped = SearchParams {
        limit: Some(10),
        seed_ids: Some(vec![resource_a.id.into()]),
        graph_depth: Some(2),
        ..SearchParams::default()
    };
    let unscoped_ids: Vec<uuid::Uuid> = app
        .client
        .search()
        .search_with_params(&unscoped)
        .await
        .expect("unscoped search")
        .iter()
        .map(|r| r.resource_id)
        .collect();
    assert!(
        unscoped_ids.contains(&resource_d.id.into()),
        "Foreign Doc should be graph-reachable without a context filter. Got: {unscoped_ids:?}"
    );

    // Scoping to scope-a excludes D even though it is graph-reachable.
    let scoped = SearchParams {
        context_ref: Some("@me/scope-a".into()),
        ..unscoped.clone()
    };
    let scoped_ids: Vec<uuid::Uuid> = app
        .client
        .search()
        .search_with_params(&scoped)
        .await
        .expect("scoped search")
        .iter()
        .map(|r| r.resource_id)
        .collect();
    assert!(
        scoped_ids.contains(&resource_a.id.into()),
        "Anchor Doc (in scope-a) should surface under its own context. Got: {scoped_ids:?}"
    );
    assert!(
        !scoped_ids.contains(&resource_d.id.into()),
        "Foreign Doc (scope-b) must NOT surface when scoped to scope-a. Got: {scoped_ids:?}"
    );

    // An unresolvable context ref errors (strict resolution), not empty results.
    let unknown = SearchParams {
        context_ref: Some(format!("@me/does-not-exist-{}", uuid::Uuid::new_v4())),
        ..unscoped.clone()
    };
    let unknown_result = app.client.search().search_with_params(&unknown).await;
    assert!(
        unknown_result.is_err(),
        "An unresolvable context_ref should error, not return results. Got: {unknown_result:?}"
    );
}
