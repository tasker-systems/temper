#![cfg(feature = "test-db")]
//! E2e coverage for the edges endpoint and for anchor-scoped search through the real stack.
//!
//! **The graph-expansion tests that gave this file its name are gone**, with the graph arm they
//! exercised (task `019fd25e`). `/api/search` no longer expands across edges, auto-seeds, or accepts
//! `graph_depth`/`no_graph`/`seed_ids`; following an edge is a composition on `/api/query`, where the
//! caller says what they want followed instead of the server guessing.
//!
//! What survives here is what was never about the graph arm: the `/api/resources/{id}/edges` read,
//! and the proof that an anchor scope actually bounds the corpus.

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
        embedded_with: None,
    }];

    IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://e2e/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(
            format!("{slug}-body-hash-{pad}", pad = "0".repeat(64))[..64].to_string(),
        ),
        content: format!("# {title}\n\n{title} content for testing."),
        metadata: None,
        managed_meta: None,
        open_meta: Some(json!({"date": "2026-04-11"})),
        chunks_packed: Some(pack_chunks(&chunks).expect("pack")),
        act: Default::default(),
        sources: Vec::new(),
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

    // Both documents carry the same distinctive term, so nothing but the scope can separate them.
    let ids_of = |r: &temper_core::types::api::SearchResponse| -> Vec<uuid::Uuid> {
        r.exact.hits.iter().map(|h| h.resource_id).collect()
    };

    let unscoped = SearchParams {
        query: Some("testing".into()),
        limit: Some(10),
        ..SearchParams::default()
    };
    let unscoped_ids = ids_of(
        &app.client
            .search()
            .search_with_params(&unscoped)
            .await
            .expect("unscoped search"),
    );
    assert!(
        unscoped_ids.contains(&resource_d.id.into()),
        "Foreign Doc must be visible without a scope. Got: {unscoped_ids:?}"
    );

    // Scoping to scope-a excludes D.
    let scoped = SearchParams {
        context_ref: Some("@me/scope-a".into()),
        ..unscoped.clone()
    };
    let scoped_ids = ids_of(
        &app.client
            .search()
            .search_with_params(&scoped)
            .await
            .expect("scoped search"),
    );
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
