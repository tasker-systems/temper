// tests/embed_job.rs — requires artifact loaded. After embedding, seeded resources have current
// chunks with embeddings.
//
// #[ignore]d in Plan 2: the current seed authors NO block content and the `block_text` source table
// does not exist until Plan 3 T1. Un-gate (remove #[ignore]) in Plan 3 once `block_text` + content
// are seeded. Do NOT seed a stand-in here (that's Plan 3's decision). Running it also needs the
// ONNX runtime present (temper-ingest `embed` feature → bge-768).
#[tokio::test]
#[ignore = "waits on Plan 3 T1: block_text source table + seeded block content"]
async fn embeds_content_blocks_into_chunks() {
    let pool = temper_next::substrate::connect().await.unwrap();
    temper_next::embed::embed_all_blocks(&pool).await.unwrap();
    let row = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_chunks WHERE embedding IS NOT NULL AND is_current",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row > 0, "expected embedded chunks after the embed job");
}
