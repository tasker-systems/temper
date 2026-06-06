// tests/embed_job.rs — requires the temper_next artifact loaded (Plan 1 + seed) and the ONNX
// runtime present (temper-ingest `embed` feature → bge-768). Job A embeds every current chunk's
// authored kb_chunk_content and writes kb_chunks.embedding (the column the seed leaves empty).
#[tokio::test]
async fn embeds_chunk_content_into_kb_chunks() {
    let pool = temper_next::substrate::connect().await.unwrap();
    let before = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_chunks WHERE embedding IS NOT NULL AND is_current",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    temper_next::embed::embed_chunks(&pool).await.unwrap();
    let after = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_chunks WHERE embedding IS NOT NULL AND is_current",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        after > before,
        "embed job must populate kb_chunks.embedding for current chunks (before={before}, after={after})"
    );
}
