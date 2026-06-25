#![cfg(feature = "artifact-tests")]
//! D2 acceptance: each charter's smoke runbook materializes into a non-degenerate, reproducible,
//! lens-sensitive shape — the model holds across the diverse corpus.
mod common;

use std::path::Path;
use temper_substrate::scenario::{bootseed, model::Scenario, runner};
use temper_substrate::substrate;

async fn run_smoke(file: &str) {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let path = format!(
        "{}/tests/fixtures/scenarios/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let scenario: Scenario =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let base = Path::new(&path).parent().unwrap();
    runner::run_scenario(&pool, &scenario, base)
        .await
        .unwrap_or_else(|e| panic!("{file} smoke runbook failed: {e:#}"));
    temper_substrate::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger roundtrip");
}

#[tokio::test]
async fn storyteller_smoke() {
    run_smoke("storyteller-smoke.yaml").await
}

#[tokio::test]
async fn temper_convergence_smoke() {
    run_smoke("temper-convergence-smoke.yaml").await
}

#[tokio::test]
async fn temper_foundational_smoke() {
    run_smoke("temper-foundational-smoke.yaml").await
}

#[tokio::test]
async fn learning_maths_smoke() {
    run_smoke("learning-maths-smoke.yaml").await
}

#[tokio::test]
async fn l0_kernel_orientation_smoke() {
    run_smoke("l0-kernel-orientation.yaml").await
}
