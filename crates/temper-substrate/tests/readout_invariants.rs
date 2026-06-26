#![cfg(feature = "artifact-tests")]
//! Ported from the retired legacy read-path tests: the readout regression guards (no NULL
//! content_cohesion, no NaN salience/telos_alignment, >=2 emergent regions) and the embed
//! completeness invariant — now over the YAML onboarding seed on an ephemeral DB.
mod common;

use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{embed, write};

const ONBOARDING_SEED: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/seeds/onboarding-cogmap.yaml");

fn seed() -> Seed {
    serde_yaml::from_str(&std::fs::read_to_string(ONBOARDING_SEED).unwrap()).unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn materialize_populates_finite_readouts_and_multiple_regions(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();
    let first = write::materialize_cogmap(&pool, loaded.cogmap, "telos-default", loaded.emitter)
        .await
        .unwrap();
    let second = write::materialize_cogmap(&pool, loaded.cogmap, "telos-default", loaded.emitter)
        .await
        .unwrap();
    assert_eq!(first.membership_fingerprint, second.membership_fingerprint, "reproducible");
    assert!(first.regions >= 2, "expected >=2 emergent regions on the enriched seed");

    let nulls = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_cogmap_regions WHERE content_cohesion IS NULL AND NOT is_folded",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nulls, 0, "all live regions have a computed content_cohesion");

    let nan = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_cogmap_regions WHERE NOT is_folded \
         AND (salience = 'NaN'::float8 OR telos_alignment = 'NaN'::float8)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nan, 0, "no live region may have NaN salience or telos_alignment");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn embed_leaves_no_current_chunk_unembedded(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let unembedded = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_chunks ch \
         JOIN kb_chunk_content cc ON cc.chunk_id = ch.id \
         JOIN kb_content_blocks b ON b.id = ch.block_id \
         WHERE ch.is_current AND NOT b.is_folded AND ch.embedding IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(unembedded, 0, "embed job must leave no current chunk with content unembedded");

    let embedded = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_chunks WHERE embedding IS NOT NULL AND is_current",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(embedded > 0, "expected >=1 embedded current chunk");
}
