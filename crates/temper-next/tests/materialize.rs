// tests/materialize.rs — requires artifact + Plan 3 enriched seed + embeddings.
//
// #[ignore]d in Plan 2: `embed_all_blocks` reads the `block_text` source table (Plan 3 T1) and the
// readout assertions need embedded chunks + the enriched cast (≥2 emergent regions). Un-gate in
// Plan 3 once the seed lands. Running it also needs the ONNX runtime (temper-ingest `embed`).
#[tokio::test]
#[ignore = "waits on Plan 3: enriched seed (block_text + content + ≥2-region cast) and embeddings"]
async fn materialize_is_reproducible_and_populates_readouts() {
    let pool = temper_next::substrate::connect().await.unwrap();
    temper_next::embed::embed_all_blocks(&pool).await.unwrap();
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
