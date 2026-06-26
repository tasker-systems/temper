#![cfg(feature = "artifact-tests")]
//! Load-path equivalence: a seed loaded STANDALONE (parse the seed document, `load_seed`,
//! materialize) and the SAME seed loaded THROUGH A SCENARIO (a path-referencing scenario whose
//! runbook is one materialize) must produce identical region membership. The split is real — two
//! document kinds — but the load path is one code path by construction; this test pins that.
//!
//! Membership is compared by the canonical origin_uri-keyed partition signature
//! (`common::telos_default_partition`), NOT `membership_fingerprint`: identity-as-input regenerates
//! every UUID per instantiation, so the raw fingerprint is only comparable within one load.
//!
//! The standalone path also re-runs the replay proof (payload spec §7) unchanged: the replay
//! machinery introduced for the scenario path must hold for a seed instantiated with no runbook.
//!
//! ONNX-dependent. Isolated ephemeral DB via `temper_substrate::MIGRATOR`.

mod common;

use std::path::Path;
use temper_substrate::scenario::model::{Scenario, Seed, SeedRef, Step};
use temper_substrate::scenario::{bootseed, loader, runner};
use temper_substrate::{embed, replay, substrate, write};

const SEED_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/onboarding-cogmap.yaml"
);

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn seed_standalone_and_via_scenario_memberships_match(pool: sqlx::PgPool) {
    // Path A — standalone: parse the seed document, load it, one telos-default materialize.
    common::reset_schema(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();

    let seed: Seed = serde_yaml::from_str(&std::fs::read_to_string(SEED_PATH).unwrap()).unwrap();
    let loaded = loader::load_seed(&pool, &seed).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();
    write::materialize_cogmap(&pool, loaded.cogmap, "telos-default", loaded.emitter)
        .await
        .unwrap();
    let sig_standalone = common::telos_default_partition(&pool, loaded.cogmap).await;

    // The replay proof runs unchanged over the standalone-seed path: snapshot the ledger, reset to a
    // clean namespace, walk the same _project_* halves, and the projections come back byte-identical.
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    for ((table_a, a), (table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(
            a, b,
            "projection table {table_a} diverged under replay of a standalone seed"
        );
    }

    // Path B — through a scenario: the same seed document referenced by path, runbook = one
    // materialize of the same lens.
    common::reset_schema(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();

    let seed_dir = Path::new(SEED_PATH).parent().unwrap();
    let scenario = Scenario {
        name: "onboarding-cogmap-equivalence".into(),
        seed: SeedRef::Path("onboarding-cogmap.yaml".into()),
        steps: vec![Step::Materialize {
            lens: "telos-default".into(),
        }],
    };
    runner::run_scenario(&pool, &scenario, seed_dir)
        .await
        .unwrap();
    let cogmap = substrate::cogmap_by_name(&pool, "onboarding-cogmap")
        .await
        .unwrap();
    let sig_via_scenario = common::telos_default_partition(&pool, cogmap).await;

    assert_eq!(
        sig_standalone, sig_via_scenario,
        "standalone-seed and via-scenario load paths must produce identical region membership"
    );
}
