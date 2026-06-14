#![cfg(feature = "artifact-tests")]
//! WS6 chunk 3 — parity-read harness. Each test ports one production read to `temper_next.*` and
//! asserts identical output for the same logical query over the synthesized prod-shape fixture.
//! Isolated ephemeral DB per test via `#[sqlx::test(migrator = ...)]` (NOT the psql-reset write-path
//! group). Runtime schema-qualified reads throughout (same discipline as `synthesis::source`).
mod common;

use common::fixture_ids;
use temper_next::readback::ResolvedIds;

/// Smoke test: the chunk-3 harness composes. Seed the prod-shape fixture into `public.*`, synthesize
/// into `temper_next.*`, then build the `old↔new` id bimap by `origin_uri`. The synthesized id set is
/// the 4 ACTIVE fixture resources (R4 `temper://fixture/deleted-doc` is excluded, §0 active-only), and
/// the bimap round-trips for a known fixture resource.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn parity_harness_setup_synthesizes(pool: sqlx::PgPool) {
    common::seed_and_synthesize(&pool).await;

    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");

    let new_ids: Vec<_> = ids.new_ids().collect();
    assert!(!new_ids.is_empty(), "synthesized id set is non-empty");
    assert_eq!(
        ids.len(),
        4,
        "4 active fixture resources synthesized (R4 deleted-doc excluded, §0 active-only)"
    );

    // The bimap round-trips for a known fixture resource (R2, the task).
    let new = ids
        .to_new(fixture_ids::RESOURCE_TASK)
        .expect("R2 (task) has a synthesized id");
    assert_eq!(
        ids.to_old(new),
        Some(fixture_ids::RESOURCE_TASK),
        "old→new→old round-trips for R2"
    );
    assert_eq!(
        ids.origin_uri_for_new(new),
        Some("temper://fixture/task-doc"),
        "the synthesized id resolves back to R2's origin_uri"
    );
}
