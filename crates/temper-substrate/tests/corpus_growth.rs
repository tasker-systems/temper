#![cfg(feature = "artifact-tests")]
//! D3 acceptance: the growth runbooks exhibit real region drift — material arrives, a relationship
//! folds, and region membership demonstrably changes across materializes. This is the substrate WS5
//! drift detection tests `incremental ≡ full` against.
mod common;

use std::path::Path;
use temper_substrate::scenario::{bootseed, model::Scenario, runner};

async fn run_growth(pool: &sqlx::PgPool, file: &str) {
    bootseed::seed_system(pool).await.unwrap();
    let path = format!(
        "{}/tests/fixtures/scenarios/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let scenario: Scenario =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let base = Path::new(&path).parent().unwrap();
    runner::run_scenario(pool, &scenario, base)
        .await
        .unwrap_or_else(|e| panic!("{file} growth runbook failed: {e:#}"));
    temper_substrate::payloads::verify_ledger_roundtrip(pool)
        .await
        .expect("ledger roundtrip");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn learning_maths_growth(pool: sqlx::PgPool) {
    run_growth(&pool, "learning-maths-growth.yaml").await
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn storyteller_growth(pool: sqlx::PgPool) {
    run_growth(&pool, "storyteller-growth.yaml").await
}
