#![cfg(feature = "artifact-tests")]
//! D3 acceptance: the growth runbooks exhibit real region drift — material arrives, a relationship
//! folds, and region membership demonstrably changes across materializes. This is the substrate WS5
//! drift detection tests `incremental ≡ full` against.
mod common;

use std::path::Path;
use temper_substrate::scenario::{bootseed, model::Scenario, runner};
use temper_substrate::substrate;

async fn run_growth(file: &str) {
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
        .unwrap_or_else(|e| panic!("{file} growth runbook failed: {e:#}"));
    temper_substrate::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger roundtrip");
}

#[tokio::test]
async fn learning_maths_growth() {
    run_growth("learning-maths-growth.yaml").await
}

#[tokio::test]
async fn storyteller_growth() {
    run_growth("storyteller-growth.yaml").await
}
