// tests/materialize.rs — requires the temper_next artifact + the Plan 3 enriched seed + the ONNX
// runtime (temper-ingest `embed` feature → bge-768). Job A embeds the seeded kb_chunk_content into
// kb_chunks.embedding, then materialize clusters the declared graph into ≥2 emergent regions and
// populates the SQL readouts.
#[tokio::test]
#[ignore = "waits on Plan 3 T2: enriched α/β cast (≥2 emergent regions); un-gate with the cast"]
async fn materialize_is_reproducible_and_populates_readouts() {
    let pool = temper_next::substrate::connect().await.unwrap();
    temper_next::embed::embed_chunks(&pool).await.unwrap();
    let cogmap = temper_next::substrate::cogmap_by_name(&pool, "onboarding-cogmap")
        .await
        .unwrap();
    let first = temper_next::write::materialize_cogmap(&pool, cogmap, "telos-default")
        .await
        .unwrap();
    let second = temper_next::write::materialize_cogmap(&pool, cogmap, "telos-default")
        .await
        .unwrap();
    assert_eq!(
        first.membership_fingerprint, second.membership_fingerprint,
        "reproducible membership"
    );
    assert!(
        first.regions >= 2,
        "expected ≥2 emergent regions on the enriched seed"
    );
    // readouts populated, not null:
    let nulls = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_cogmap_regions WHERE content_cohesion IS NULL AND NOT is_folded",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        nulls, 0,
        "all live regions have a computed content_cohesion"
    );
}
