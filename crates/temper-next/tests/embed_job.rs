#![cfg(feature = "artifact-tests-legacy")]
// tests/embed_job.rs — requires the temper_next artifact loaded (Plan 1 + seed) and the ONNX
// runtime present (temper-ingest `embed` feature → bge-768). Job A embeds every current chunk's
// authored kb_chunk_content and writes kb_chunks.embedding (the column the seed leaves empty).
//
// Idempotent assertion (the job may run against an already-embedded DB): after the job, NO current,
// non-folded chunk that has content is left without an embedding, and at least one is embedded.
#[tokio::test]
async fn embeds_chunk_content_into_kb_chunks() {
    let pool = temper_next::substrate::connect().await.unwrap();
    temper_next::embed::embed_chunks(&pool).await.unwrap();

    let unembedded = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_chunks ch \
         JOIN kb_chunk_content cc ON cc.chunk_id = ch.id \
         JOIN kb_content_blocks b ON b.id = ch.block_id \
         WHERE ch.is_current AND NOT b.is_folded AND ch.embedding IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        unembedded, 0,
        "embed job must leave no current chunk with content unembedded"
    );

    let embedded = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_chunks WHERE embedding IS NOT NULL AND is_current",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(embedded > 0, "expected ≥1 embedded current chunk");
}
