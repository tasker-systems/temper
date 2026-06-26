#![cfg(feature = "artifact-tests")]
//! Corpus sweep: EVERY seed document in `tests/fixtures/seeds/` (excluding `system.yaml`, the
//! boot-seed) parses as a `Seed`, loads through the standard path (`bootseed` + `load_seed`), and
//! its telos charter reproduces byte-exact through the role-filtered `resource_blocks` reads.
//!
//! The onboarding-specific proofs (cross-path membership equivalence, S6 expectations) stay in
//! their own tests — they prove machinery. This sweep proves charters, and grows automatically as
//! corpus seeds land (charter-bootstrapping design, §5).
//!
//! Resets the artifact per seed, ONNX-dependent, serialized via the temper-substrate-write group.

mod common;

use temper_substrate::scenario::model::Seed;
use temper_substrate::scenario::{bootseed, loader};

const SEEDS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/seeds");

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn every_corpus_seed_loads_and_charter_roundtrips(pool: sqlx::PgPool) {
    let mut paths: Vec<_> = std::fs::read_dir(SEEDS_DIR)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .filter(|p| p.file_name().is_some_and(|n| n != "system.yaml"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no corpus seeds found in {SEEDS_DIR}");

    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let seed: Seed = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: does not parse as a Seed: {e}"));

        common::reset_schema(&pool).await;
        bootseed::seed_system(&pool).await.unwrap();
        let loaded = loader::load_seed(&pool, &seed)
            .await
            .unwrap_or_else(|e| panic!("{name}: load_seed failed: {e}"));
        let telos = loaded.keys["telos"];

        // The charter comes back exactly as authored: per role, in order, byte-equal.
        let expected = seed.cogmap.telos.block_specs();
        for role in ["statement", "question", "framing"] {
            let want: Vec<String> = expected
                .iter()
                .filter(|(r, _)| *r == role)
                .map(|(_, body)| body.clone())
                .collect();
            let got: Vec<String> = sqlx::query_scalar(
                "SELECT body_text FROM resource_blocks($1, 'cogmap', $2, $3) ORDER BY seq",
            )
            .bind(telos)
            .bind(loaded.cogmap)
            .bind(role)
            .fetch_all(&pool)
            .await
            .unwrap();
            assert_eq!(
                got, want,
                "{name}: role '{role}' blocks diverge from the YAML"
            );
        }

        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM resource_blocks($1, 'cogmap', $2, NULL)")
                .bind(telos)
                .bind(loaded.cogmap)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            total as usize,
            expected.len(),
            "{name}: charter block count diverges from the YAML"
        );
    }
}
