//! Regression guard for issue #297 — the server embeds a text-only search query.
//!
//! Before the fix, only the CLI ran the vector arm, because it computed the query embedding
//! client-side and passed it in `SearchParams.embedding`. Every server-side surface (MCP, raw
//! `POST /api/search`, agent workers) sent text only, so `search_select` ran FTS + graph and the
//! vector arm was dead: `vec_norm` was always 0.0 and a resource whose only signal was semantic
//! (no lexical match) vanished from results entirely.
//!
//! `search_select` now embeds the query server-side when the caller sent text but no vector, using
//! the SAME plain `embed_text` path the corpus was ingested with. These tests drive the exact
//! text-only path (`text_query` → `POST /api/search` with `embedding: None`) that MCP and HTTP use.
//!
//! `test-embed` gated: they need the real ONNX model both to ingest chunks with true embeddings and
//! for the server to embed the query. The Embed & MCP Round-Trip CI job runs these; locally use
//! `cargo make test-e2e-embed`.
#![cfg(all(feature = "test-db", feature = "test-embed"))]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};

/// Ingest a resource whose chunks carry REAL bge embeddings (via the same `prepare_markdown` path the
/// corpus is ingested with), so the vector arm has a meaningful vector space to match against.
async fn ingest_semantic(
    app: &common::E2eTestApp,
    title: &str,
    slug: &str,
    content: &str,
    context_name: &str,
) {
    let packed = temper_ingest::pipeline::prepare_markdown(content).expect("prepare_markdown");
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://sem/{slug}"),
        context_ref: format!("@me/{context_name}"),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(content)),
        content: content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: Some(serde_json::json!({"date": "2026-07-07"})),
        chunks_packed: Some(pack_chunks(&packed).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");
}

/// A text-only search (no client-supplied embedding — the MCP / HTTP path) returns hits with non-zero
/// `vec_norm`, and a semantic-only resource — one that shares NO query terms, so its `fts_norm`
/// is 0 — is surfaced purely on its vector score. Before #297 that row vanished and every hit scored
/// `vec_norm: 0.0`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn server_embeds_text_only_query_surfaces_semantic_only_hit(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("sem", None)
        .await
        .expect("context create");

    // Resource A — lexically AND semantically matches "kubernetes deployment".
    ingest_semantic(
        &app,
        "Kubernetes Deployment Guide",
        "k8s-deploy-guide",
        "This guide covers kubernetes deployment: rolling updates, blue-green cutovers, and canary releases.",
        "sem",
    )
    .await;

    // Resource B — the vanishing row: same TOPIC, but its title and body contain none of the query
    // terms ("kubernetes", "deployment"), so `plainto_tsquery` cannot match it. Its only signal is
    // semantic proximity in the embedding space.
    ingest_semantic(
        &app,
        "Container Scheduling Primer",
        "container-scheduling-primer",
        "Pods, replicas, and self-healing workloads are placed and rescheduled automatically by the control plane.",
        "sem",
    )
    .await;

    // The MCP / HTTP path: query text only, `embedding: None`. The server must embed it now.
    let params = temper_core::types::api::SearchParams {
        query: Some("kubernetes deployment".into()),
        context_ref: Some("@me/sem".into()),
        limit: Some(10),
        ..Default::default()
    };
    let resp = app
        .client
        .search()
        .search_with_params(&params)
        .await
        .expect("text search failed");

    // THE POINT OF THIS TEST, restated for two arms: the caller sent no embedding, so the wide arm
    // can only run if the SERVER embedded the query. An empty wide arm here means it did not.
    assert!(
        !resp.wide.degraded,
        "the server had to embed a text-only query and could not: {:?}",
        resp.wide.hint
    );
    assert!(
        !resp.wide.hits.is_empty(),
        "the wide arm must answer a text-only query once the server embeds it"
    );

    // The vanishing row is back. It shares NO terms with the query, so the exact arm cannot see it
    // at all — which is now visible as absence from one arm rather than as a zero in a blended row.
    let title = "Container Scheduling Primer";
    assert!(
        resp.wide.hits.iter().any(|r| r.resource.title == title),
        "the semantic-only resource must appear in the WIDE arm; got {:?}",
        resp.wide
            .hits
            .iter()
            .map(|r| (r.resource.title.as_str(), r.vec_norm))
            .collect::<Vec<_>>()
    );
    assert!(
        !resp.exact.hits.iter().any(|r| r.resource.title == title),
        "the semantic-only resource shares no query terms, so the exact arm must not carry it"
    );
    let semantic_only = resp
        .wide
        .hits
        .iter()
        .find(|r| r.resource.title == title)
        .expect("checked above");
    assert!(
        semantic_only.vec_norm > 0.0,
        "a wide hit is carried entirely by its own quantity; got {}",
        semantic_only.vec_norm
    );
}

/// Parity: the vector the server computes for a query string is identical to the one the CLI's
/// `embed_query` produces for the same string (same model, same plain `embed_text` preprocessing —
/// no BGE query prefix on either side). Guards against a future query-side prefix drifting the two
/// clients into different vector spaces.
#[test]
fn server_query_embedding_matches_cli_embed_query() {
    let text = "kubernetes deployment rollout strategy";
    let server = temper_ingest::embed::embed_text(text).expect("server embed");
    let cli = temper_cli::actions::search::embed_query(text).expect("cli embed_query");
    assert_eq!(
        server.len(),
        temper_ingest::embed::EMBEDDING_DIM,
        "server embedding is 768-dim"
    );
    assert_eq!(
        server, cli,
        "server-side and CLI query embeddings must be byte-for-byte identical (same vector space)"
    );
}
