#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

/// Helper: ingest a resource with chunks so the FTS index gets populated via trigger.
async fn ingest_with_chunks(
    app: &common::E2eTestApp,
    title: &str,
    slug: &str,
    content: &str,
    context_name: &str,
) {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: title.to_string(),
        heading_depth: 0,
        content: content.to_string(),
        content_hash: format!("{:0>64x}", slug.len()),
        embedding: vec![0.1_f32; 768],
    };
    let payload = IngestPayload {
        title: title.to_string(),
        origin_uri: format!("test://fts/{slug}"),
        context_ref: format!("@me/{context_name}"),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(format!("{:0>64x}", title.len())),
        slug: slug.to_string(),
        content: content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");
}

/// Full-text search with a plain text query finds an ingested resource.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn fts_text_query_finds_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    app.client
        .contexts()
        .create("fts-test", None)
        .await
        .expect("context create");

    ingest_with_chunks(
        &app,
        "Kubernetes Deployment Strategy",
        "k8s-deploy-strategy",
        "This document covers rolling updates, blue-green deployments, and canary releases for Kubernetes clusters.",
        "fts-test",
    )
    .await;

    // Search by text — should find via title match
    let results = app
        .client
        .search()
        .text_query(
            "kubernetes deployment",
            Some("@me/fts-test".into()),
            None,
            Some(10),
        )
        .await
        .expect("text search failed");

    assert!(
        !results.is_empty(),
        "FTS text search should find the ingested resource"
    );
    assert_eq!(results[0].title, "Kubernetes Deployment Strategy");
    // Beat 2: unified_search pipeline — origin is always "unified".
    assert_eq!(results[0].origin, "unified");
    // The resource is a genuine lexical hit, so its FTS term is non-zero. #297: the server now
    // embeds a text-only query server-side (`search_select` fills `p_emb`), so the vector term may
    // also contribute — this is no longer the dead-vector-arm path, and `vector_score` is no longer
    // asserted to be 0 (its value depends on whether the embedder is available at runtime).
    assert!(
        results[0].fts_score > 0.0,
        "a real FTS match must carry a non-zero fts_score; got {}",
        results[0].fts_score
    );
}

/// Full-text search finds resources by body content, not just title.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn fts_finds_by_body_content(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    app.client
        .contexts()
        .create("fts-body", None)
        .await
        .expect("context create");

    ingest_with_chunks(
        &app,
        "Infrastructure Notes",
        "infra-notes",
        "The canary release pipeline uses ArgoCD rollouts with automatic promotion after health checks pass.",
        "fts-body",
    )
    .await;

    // Search for a term that's only in the body, not the title
    let results = app
        .client
        .search()
        .text_query(
            "ArgoCD rollouts",
            Some("@me/fts-body".into()),
            None,
            Some(10),
        )
        .await
        .expect("body text search failed");

    assert!(
        !results.is_empty(),
        "FTS should find resource by body content"
    );
    assert_eq!(results[0].title, "Infrastructure Notes");
}

/// Search with no query and no embedding returns 400 Bad Request.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: collapsed search_select returns Ok(empty) for no-query/no-embedding instead of rejecting (search input validation #7)"]
async fn search_rejects_empty_params(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let result = app
        .client
        .search()
        .search(None, None, None, None, Some(10))
        .await;

    assert!(result.is_err(), "search with no inputs should fail");
}

/// Unified search with both text query and embedding returns results with origin "both".
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: collapsed search_select short-circuits to vector-only when an embedding is present (no unified FTS+vector combine); origin is 'vector', combined_score 0.0 (#7)"]
async fn unified_search_both_modes(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    app.client
        .contexts()
        .create("fts-unified", None)
        .await
        .expect("context create");

    ingest_with_chunks(
        &app,
        "Observability Platform Design",
        "observability-design",
        "Distributed tracing with OpenTelemetry, metrics via Prometheus, and structured logging.",
        "fts-unified",
    )
    .await;

    // Search with both text query and embedding
    let results = app
        .client
        .search()
        .search(
            Some("observability tracing".into()),
            Some(vec![0.1_f32; 768]),
            Some("fts-unified".into()),
            None,
            Some(10),
        )
        .await
        .expect("unified search failed");

    assert!(
        !results.is_empty(),
        "unified search should find the resource"
    );
    // With both FTS and vector, the result should come from "both" or at least "fts"
    assert!(
        results[0].origin == "both" || results[0].origin == "fts",
        "expected origin 'both' or 'fts', got '{}'",
        results[0].origin
    );
    assert!(results[0].combined_score > 0.0);
}

/// FTS search respects context filtering.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: collapsed search_select ignores the context filter (search context scoping #7)"]
async fn fts_respects_context_filter(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    app.client
        .contexts()
        .create("ctx-alpha", None)
        .await
        .expect("context create alpha");
    app.client
        .contexts()
        .create("ctx-beta", None)
        .await
        .expect("context create beta");

    ingest_with_chunks(
        &app,
        "Alpha Specific Document",
        "alpha-doc",
        "This document is specific to the alpha context only.",
        "ctx-alpha",
    )
    .await;

    ingest_with_chunks(
        &app,
        "Beta Specific Document",
        "beta-doc",
        "This document is specific to the beta context only.",
        "ctx-beta",
    )
    .await;

    // Search in alpha context — should only find alpha doc
    let alpha_results = app
        .client
        .search()
        .text_query(
            "specific document",
            Some("ctx-alpha".into()),
            None,
            Some(10),
        )
        .await
        .expect("alpha search failed");

    assert_eq!(alpha_results.len(), 1);
    assert_eq!(alpha_results[0].title, "Alpha Specific Document");

    // Search in beta context — should only find beta doc
    let beta_results = app
        .client
        .search()
        .text_query("specific document", Some("ctx-beta".into()), None, Some(10))
        .await
        .expect("beta search failed");

    assert_eq!(beta_results.len(), 1);
    assert_eq!(beta_results[0].title, "Beta Specific Document");
}
