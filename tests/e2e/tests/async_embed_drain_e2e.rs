//! Async-embedding drain, end-to-end through the real server (issue #299, Phase 4).
//!
//! With `TEMPER_ASYNC_EMBED=1`, a server-computed create (content only, no caller-supplied
//! `chunks_packed`) defers the vector off the request path: chunk text + FTS land synchronously and an
//! embed job is queued. This test drives that whole arc through the in-process Axum server + a real
//! Postgres:
//!   1. the deferred create returns a fully-formed resource (no bimodal / poll-for-UUID contract);
//!   2. it is FTS-findable immediately, while its current chunks carry NULL embeddings;
//!   3. its derived `embedding_status` is `pending` (design §8);
//!   4. one `dispatch_tick` backfills every deferred chunk;
//!   5. after the drain, no chunk is NULL and `embedding_status` flips to `ready`.
//!
//! The substrate round-trip (`deferred_create_is_fts_immediate_then_backfills_vectors`) proves the
//! write/backfill primitive in isolation; this is the surface/e2e half — the DbBackend defer gate, the
//! enqueue, the derivation, and the drain composed against the HTTP create + search path.
//!
//! `test-embed` gated: the server computes real bge embeddings when it defers *and* when it drains, so
//! ONNX Runtime must be present. The "Embed & MCP Round-Trip" CI job runs this; locally use
//! `cargo make test-e2e-embed`. This is the only test in its (per-file) test binary, so setting the
//! `TEMPER_ASYNC_EMBED` gate via `set_var` never races another test.
#![cfg(all(feature = "test-db", feature = "test-embed"))]

mod common;

use temper_core::types::ingest::IngestPayload;
use temper_core::types::workflow_job::EmbeddingStatus;
use temper_services::services::embed_service;

/// Count current, non-folded chunks of `resource` whose embedding is still NULL (deferred).
async fn null_current_chunks(pool: &sqlx::PgPool, resource: uuid::Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) \
           FROM kb_chunks ch \
           JOIN kb_content_blocks b ON b.id = ch.block_id \
          WHERE ch.resource_id = $1 AND ch.is_current AND NOT b.is_folded AND ch.embedding IS NULL",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn status_of(pool: &sqlx::PgPool, resource: uuid::Uuid) -> EmbeddingStatus {
    *embed_service::embedding_status_batch(pool, &[resource])
        .await
        .unwrap()
        .get(&resource)
        .expect("status derived for the resource")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn deferred_create_is_fts_immediate_then_ready_after_drain(pool: sqlx::PgPool) {
    // This test's own binary (one test per file) — the process-wide env set is race-free here.
    std::env::set_var("TEMPER_ASYNC_EMBED", "1");

    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("ae", None)
        .await
        .expect("context create");

    // A content-only create (no `chunks_packed`): the server computes the embedding, and under
    // TEMPER_ASYNC_EMBED it DEFERS it — chunk text + FTS now, vector later.
    let content = "The kubernetes deployment guide covers rolling updates and canary releases.";
    let payload = IngestPayload {
        title: "Deferred Embed Doc".to_string(),
        origin_uri: "test://ae/deferred".to_string(),
        context_ref: "@me/ae".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(content)),
        slug: "deferred-embed-doc".to_string(),
        content: content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        sources: Vec::new(),
        act: Default::default(),
    };
    let created = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("deferred create failed");

    // (1) The create contract is uniform — a fully-formed resource, never a poll-for-UUID stub.
    assert_eq!(
        created.title, "Deferred Embed Doc",
        "create returns the formed resource"
    );
    let id = uuid::Uuid::from(created.id);

    // (2) FTS is immediate: a lexical query finds the resource even with no vector yet.
    let fts = app
        .client
        .search()
        .text_query(
            "kubernetes deployment",
            Some("@me/ae".into()),
            None,
            Some(10),
        )
        .await
        .expect("text search failed");
    assert!(
        fts.iter().any(|r| r.resource_id == id),
        "deferred create is FTS-findable immediately; got {:?}",
        fts.iter().map(|r| r.title.as_str()).collect::<Vec<_>>()
    );

    // (3) Chunks landed unembedded, and the derived status is `pending` (NULL chunks + a live job).
    let null_before = null_current_chunks(&app.pool, id).await;
    assert!(
        null_before >= 1,
        "deferred create writes NULL-embedding chunks"
    );
    assert_eq!(status_of(&app.pool, id).await, EmbeddingStatus::Pending);

    // (4) One drain pass backfills every deferred chunk.
    let summary = embed_service::dispatch_tick(&app.pool, None, false)
        .await
        .expect("dispatch_tick");
    assert!(
        summary.completed >= 1,
        "the queued embed job is drained: {summary:?}"
    );
    assert!(
        summary.chunks_embedded >= 1,
        "at least one chunk embedded: {summary:?}"
    );

    // (5) After the drain nothing is NULL and the status flips to `ready`.
    assert_eq!(
        null_current_chunks(&app.pool, id).await,
        0,
        "all chunks embedded post-drain"
    );
    assert_eq!(status_of(&app.pool, id).await, EmbeddingStatus::Ready);
}
