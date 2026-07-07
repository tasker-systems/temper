//! Regression guard for issue #297 — the server embeds a text-only search query.
//!
//! Before the fix, only the CLI ran the vector arm, because it computed the query embedding
//! client-side and passed it in `SearchParams.embedding`. Every server-side surface (MCP, raw
//! `POST /api/search`, agent workers) sent text only, so `search_select` ran FTS + graph and the
//! vector arm was dead: `vector_score` was always 0.0 and a resource whose only signal was semantic
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
        title: title.to_string(),
        origin_uri: format!("test://sem/{slug}"),
        context_ref: format!("@me/{context_name}"),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(content)),
        slug: slug.to_string(),
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
/// `vector_score`, and a semantic-only resource — one that shares NO query terms, so its `fts_score`
/// is 0 — is surfaced purely on its vector score. Before #297 that row vanished and every hit scored
/// `vector_score: 0.0`.
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
    let results = app
        .client
        .search()
        .text_query(
            "kubernetes deployment",
            Some("@me/sem".into()),
            None,
            Some(10),
        )
        .await
        .expect("text search failed");

    assert!(!results.is_empty(), "text-only search should return hits");

    // Every hit is scored by the vector arm now — it is no longer dead.
    assert!(
        results.iter().any(|r| r.vector_score > 0.0),
        "at least one hit must carry a non-zero vector_score once the server embeds the query; \
         got {:?}",
        results
            .iter()
            .map(|r| (r.title.as_str(), r.vector_score))
            .collect::<Vec<_>>()
    );

    // The vanishing row is back: present, with fts_score 0 (no lexical match) and a real vector score.
    let semantic_only = results
        .iter()
        .find(|r| r.title == "Container Scheduling Primer")
        .unwrap_or_else(|| {
            panic!(
                "semantic-only resource must appear in results; got {:?}",
                results.iter().map(|r| r.title.as_str()).collect::<Vec<_>>()
            )
        });
    assert_eq!(
        semantic_only.fts_score, 0.0,
        "semantic-only row must have no lexical signal"
    );
    assert!(
        semantic_only.vector_score > 0.0,
        "semantic-only row must be carried entirely by its vector score; got {}",
        semantic_only.vector_score
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
