#![cfg(feature = "artifact-tests")]
//! WS5 acceptance: incremental materialization is byte-identical to a full re-materialize.
//!
//! Two independent proofs per growth runbook:
//!   1. PER-STEP — running the runbook in `Incremental` mode must satisfy every assertion the runbook
//!      encodes (region counts, co-region drift across the fold, the untouched persona region) AND
//!      pass `verify_ledger_roundtrip`. The runbook's own checks are the full-path expectations, so
//!      passing them in incremental mode proves the incremental path tracks full at every materialize.
//!   2. BYTE-IDENTICAL — the final UUID-independent region partition (origin_uri based, so comparable
//!      across separate instantiations) under incremental mode equals the one under full mode.
//!
//! The storyteller/learning-maths growth runbooks each fold an edge that re-shapes one neighborhood
//! while leaving another untouched — so each exercises both a recomputed component and a reused one.
mod common;

use std::path::Path;
use temper_next::scenario::runner::MaterializeMode;
use temper_next::scenario::{bootseed, model::Scenario, runner};
use temper_next::substrate;

/// Run a growth runbook end-to-end in the given mode against a freshly reset namespace, verify the
/// ledger roundtrips, and return the final telos-default partition signature.
async fn run_growth(file: &str, mode: MaterializeMode) -> String {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let path = format!(
        "{}/../../schema-artifact/scenarios/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let scenario: Scenario =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let base = Path::new(&path).parent().unwrap();
    runner::run_scenario_with(&pool, &scenario, base, mode)
        .await
        .unwrap_or_else(|e| panic!("{file} ({mode:?}) runbook failed: {e:#}"));
    temper_next::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger roundtrip");

    // exactly one (non-system) cogmap per growth scenario — bootseed creates only global lenses.
    let cogmaps: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM kb_cogmaps")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(cogmaps.len(), 1, "expected exactly one cogmap for {file}");
    common::telos_default_partition(&pool, cogmaps[0]).await
}

async fn assert_incremental_equals_full(file: &str) {
    let full = run_growth(file, MaterializeMode::Full).await;
    let incremental = run_growth(file, MaterializeMode::Incremental).await;
    assert_eq!(
        full, incremental,
        "{file}: incremental partition must be byte-identical to full"
    );
}

#[tokio::test]
async fn storyteller_growth_incremental_equals_full() {
    assert_incremental_equals_full("storyteller-growth.yaml").await
}

/// Byte-identical output is necessary but not sufficient: a bug that silently does a FULL recompute
/// each step would also pass. This pins the mechanism — after the storyteller fold, the untouched
/// persona region is REUSED (still bears the first materialize's event), while the recomputed
/// commitment region bears the second. So the live regions span ≥2 distinct materialize events; a
/// degenerate fold-everything path would leave them all on one.
#[tokio::test]
async fn incremental_actually_reuses_the_untouched_component() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let path = format!(
        "{}/../../schema-artifact/scenarios/storyteller-growth.yaml",
        env!("CARGO_MANIFEST_DIR")
    );
    let scenario: Scenario =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let base = Path::new(&path).parent().unwrap();
    runner::run_scenario_with(&pool, &scenario, base, MaterializeMode::Incremental)
        .await
        .unwrap();

    let distinct_events: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT r.asserted_by_event_id) \
         FROM kb_cogmap_regions r JOIN kb_cogmap_lenses l ON l.id=r.lens_id AND l.name='telos-default' \
         WHERE NOT r.is_folded",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        distinct_events >= 2,
        "expected reused + recomputed regions across ≥2 materialize events, got {distinct_events} \
         (incremental may have degenerated to a full recompute)"
    );
}

#[tokio::test]
async fn learning_maths_growth_incremental_equals_full() {
    assert_incremental_equals_full("learning-maths-growth.yaml").await
}

/// Run a scenario end-to-end in the given mode against a freshly reset namespace, verify the ledger
/// roundtrips, and return the final telos-default READOUT signature (membership + readout values).
async fn run_readout_scenario(file: &str, mode: MaterializeMode) -> String {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let path = format!(
        "{}/../../schema-artifact/scenarios/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let scenario: Scenario =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let base = Path::new(&path).parent().unwrap();
    runner::run_scenario_with(&pool, &scenario, base, mode)
        .await
        .unwrap_or_else(|e| panic!("{file} ({mode:?}) failed: {e:#}"));
    temper_next::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger roundtrip");
    let cogmaps: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM kb_cogmaps")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(cogmaps.len(), 1);
    common::telos_default_readout_signature(&pool, cogmaps[0]).await
}

/// Slice 3b acceptance: after a body REVISION (a readout-only change), incremental materialization
/// must produce the same readout values as a full recompute — it may reuse a component's membership
/// AND region ids, but it must re-run that region's readouts over the moved embedding, not serve the
/// stale ones. A regression here means incremental served a reused region's pre-revision readouts.
#[tokio::test]
async fn readout_refresh_incremental_equals_full() {
    let full = run_readout_scenario("storyteller-readout.yaml", MaterializeMode::Full).await;
    let incremental =
        run_readout_scenario("storyteller-readout.yaml", MaterializeMode::Incremental).await;
    assert_eq!(
        full, incremental,
        "after a body revision, incremental readouts must match a full recompute (not reuse stale readouts)"
    );
}
