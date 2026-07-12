#![cfg(feature = "artifact-tests")]
//! Proof obligation 2 (replay) + 4 (CAS retention), payload spec §7: run a scenario, snapshot,
//! reset the namespace, walk the ledger through the SAME _project_* halves, and prove the
//! projections come back byte-identical (masked-surrogate rule). Regions re-prove by
//! re-materialization matching the payload's recorded membership fingerprint.
//!
//! ONNX-dependent. Isolated ephemeral DB via `temper_substrate::MIGRATOR`.

mod common;

use temper_substrate::{
    replay, scenario::bootseed, scenario::model::Scenario, scenario::runner, write,
};

const ONBOARDING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/scenarios/onboarding-cogmap.yaml"
);

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_reproduces_projections_byte_identically(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();

    let scenario: Scenario =
        serde_yaml::from_str(&std::fs::read_to_string(ONBOARDING).unwrap()).unwrap();
    let base_dir = std::path::Path::new(ONBOARDING).parent().unwrap();
    runner::run_scenario(&pool, &scenario, base_dir)
        .await
        .unwrap();

    // capture: projections (the diff baseline), inputs + sidecars (the replay substrate), and the
    // recorded materialization acts (the region re-proof targets)
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    let recorded = replay::recorded_materializations(&pool).await.unwrap();
    assert!(
        !recorded.is_empty(),
        "scenario must have materialized at least once"
    );

    // reset to a clean, UN-seeded namespace; replay the ledger through the projection halves
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();

    let after = replay::dump_projections(&pool).await.unwrap();
    for ((table_a, a), (table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(a, b, "projection table {table_a} diverged under replay");
    }

    // regions: second-order derived — re-materialize and match the recorded fingerprints. A lens
    // whose substrate gained formation-affecting events AFTER its recorded watermark is legitimately
    // stale (the drift-detection concept) and is skipped; at least one lens must be provably fresh.
    let mut proven = 0usize;
    for (anchor, lens_id, watermark, fingerprint) in recorded {
        if replay::formation_touched_since(&pool, anchor, watermark)
            .await
            .unwrap()
        {
            continue; // recorded act is stale relative to the substrate — re-proof not applicable
        }
        let lens_name: String =
            sqlx::query_scalar("SELECT name FROM kb_cogmap_lenses WHERE id = $1")
                .bind(lens_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let emitter: uuid::Uuid =
            sqlx::query_scalar("SELECT id FROM kb_entities ORDER BY id LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let out = write::materialize(&pool, anchor, &lens_name, emitter.into())
            .await
            .unwrap();
        assert_eq!(
            out.membership_fingerprint, fingerprint,
            "re-materialization under lens {lens_name} must reproduce the recorded fingerprint"
        );
        proven += 1;
    }
    assert!(
        proven >= 1,
        "at least one recorded materialization must be fresh enough to re-prove"
    );
}
