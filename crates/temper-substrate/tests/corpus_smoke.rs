#![cfg(feature = "artifact-tests")]
//! D2 acceptance: each charter's smoke runbook materializes into a non-degenerate, reproducible,
//! lens-sensitive shape — the model holds across the diverse corpus.
//! Isolated ephemeral DB via `temper_substrate::MIGRATOR`.
mod common;

use std::path::Path;
use temper_substrate::scenario::{bootseed, model::Scenario, runner};

async fn run_smoke(pool: &sqlx::PgPool, file: &str) {
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
        .unwrap_or_else(|e| panic!("{file} smoke runbook failed: {e:#}"));
    temper_substrate::payloads::verify_ledger_roundtrip(pool)
        .await
        .expect("ledger roundtrip");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn storyteller_smoke(pool: sqlx::PgPool) {
    run_smoke(&pool, "storyteller-smoke.yaml").await
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn temper_convergence_smoke(pool: sqlx::PgPool) {
    run_smoke(&pool, "temper-convergence-smoke.yaml").await
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn temper_foundational_smoke(pool: sqlx::PgPool) {
    run_smoke(&pool, "temper-foundational-smoke.yaml").await
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn learning_maths_smoke(pool: sqlx::PgPool) {
    run_smoke(&pool, "learning-maths-smoke.yaml").await
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn l0_kernel_orientation_smoke(pool: sqlx::PgPool) {
    run_smoke(&pool, "l0-kernel-orientation.yaml").await
}
