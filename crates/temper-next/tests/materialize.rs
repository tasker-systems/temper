#![cfg(feature = "artifact-tests-legacy")]
// tests/materialize.rs — requires the temper_next artifact + the Plan 3 enriched seed + the ONNX
// runtime (temper-ingest `embed` feature → bge-768). Job A embeds the seeded kb_chunk_content into
// kb_chunks.embedding, then materialize clusters the declared graph into ≥2 emergent regions and
// populates the SQL readouts.
#[tokio::test]
async fn materialize_is_reproducible_and_populates_readouts() {
    let pool = temper_next::substrate::connect().await.unwrap();
    temper_next::embed::embed_chunks(&pool).await.unwrap();
    let cogmap = temper_next::substrate::cogmap_by_name(&pool, "onboarding-cogmap")
        .await
        .unwrap();
    let emitter: uuid::Uuid = sqlx::query_scalar(
        "SELECT emitter_entity_id FROM kb_events \
         WHERE producing_anchor_table='kb_cogmaps' AND producing_anchor_id=$1 \
         ORDER BY occurred_at ASC LIMIT 1",
    )
    .bind(cogmap)
    .fetch_one(&pool)
    .await
    .unwrap();
    let first = temper_next::write::materialize_cogmap(&pool, cogmap, "telos-default", emitter)
        .await
        .unwrap();
    let second = temper_next::write::materialize_cogmap(&pool, cogmap, "telos-default", emitter)
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
    // readouts must be FINITE, not NaN. (Regression guard: telos_alignment was computed in the same
    // UPDATE that set centroid, so it read the zero-vector placeholder → cosine NaN → NaN salience.)
    let nan = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_cogmap_regions WHERE NOT is_folded \
         AND (salience = 'NaN'::float8 OR telos_alignment = 'NaN'::float8)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        nan, 0,
        "no live region may have NaN salience or telos_alignment"
    );
}
