#![cfg(feature = "artifact-tests")]
//! Acceptance gate: the onboarding scenario as YAML roundtrips to the same regions + S6 verdicts as
//! run_eval.sh. Two checks, two states:
//!  - `passes_full_s6_runbook`: run the whole S6a–h runbook via run_scenario; its declarative asserts
//!    ARE the primary check (incl. S6b reproducibility, S6f plurality, S6h staleness/solo-joins-α).
//!  - `baseline_matches_04b_sql_verdict`: an INDEPENDENT encoding — the 04b verdict logic (a SQL
//!    aggregate over origin_uri) evaluated at the BASELINE state (load + one telos-default materialize,
//!    before the S6h mutation that retires solo's singleton). Same prose → same embeddings → same
//!    regions → same verdict.
//!
//! All are isolated ephemeral DBs via `temper_substrate::MIGRATOR` and are ONNX-dependent.

use std::path::Path;
use temper_substrate::scenario::{bootseed, loader, model::Scenario, model::Seed, runner};
use temper_substrate::{embed, write};

const ONBOARDING_SCENARIO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/scenarios/onboarding-cogmap.yaml"
);
const ONBOARDING_SEED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/onboarding-cogmap.yaml"
);

fn scenario_base_dir() -> &'static Path {
    Path::new(ONBOARDING_SCENARIO).parent().unwrap()
}

fn load_scenario_yaml() -> Scenario {
    serde_yaml::from_str(&std::fs::read_to_string(ONBOARDING_SCENARIO).unwrap()).unwrap()
}

fn load_seed_yaml() -> Seed {
    serde_yaml::from_str(&std::fs::read_to_string(ONBOARDING_SEED).unwrap()).unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn passes_full_s6_runbook(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    runner::run_scenario(&pool, &load_scenario_yaml(), scenario_base_dir())
        .await
        .expect("declarative S6a-h asserts pass");

    // proof obligation 1: every fired event's payload deserializes into its typed struct
    temper_substrate::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger payload roundtrip");
}

// The `v AS (...)` body of 04b_region_suite.sql, inlined as one query (S6a/c/d/e/g — the BASELINE
// region verdict). Transcribed verbatim; keyed on origin_uri so it is UUID-independent.
const VERDICT_SQL: &str = r#"
WITH td AS (
  SELECT res.origin_uri, m.region_id
  FROM kb_cogmap_region_members m
  JOIN kb_cogmap_regions r ON r.id = m.region_id AND NOT r.is_folded
  JOIN kb_cogmap_lenses  l ON l.id = r.lens_id AND l.name = 'telos-default'
  JOIN kb_resources    res ON res.id = m.member_id
)
SELECT (
  ((SELECT count(*) FROM kb_cogmap_regions r JOIN kb_cogmap_lenses l ON l.id = r.lens_id
      WHERE l.name = 'telos-default' AND NOT r.is_folded) >= 2
   AND (SELECT a.region_id = b.region_id FROM td a, td b
          WHERE a.origin_uri = 'temper://c/pair' AND b.origin_uri = 'temper://c/smallest'))
  AND (SELECT ca.content_cohesion > cb.content_cohesion FROM kb_cogmap_regions ca, kb_cogmap_regions cb
         WHERE ca.id = (SELECT region_id FROM td WHERE origin_uri = 'temper://c/pair')
           AND cb.id = (SELECT region_id FROM td WHERE origin_uri = 'temper://c/staging'))
  AND (SELECT count(*) = 1 FROM td WHERE region_id = (SELECT region_id FROM td WHERE origin_uri = 'temper://c/solo'))
  AND (SELECT (SELECT region_id FROM td WHERE origin_uri = 'temper://c/checklist')
            = (SELECT region_id FROM td WHERE origin_uri = 'temper://c/staging'))
  AND (SELECT (SELECT region_id FROM td WHERE origin_uri = 'temper://c/bluegreen')
            = (SELECT region_id FROM td WHERE origin_uri = 'temper://c/bigbang')
         AND (SELECT internal_tension FROM kb_cogmap_regions
                WHERE id = (SELECT region_id FROM td WHERE origin_uri = 'temper://c/bluegreen')) > 0)
) AS all_pass
"#;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn baseline_matches_04b_sql_verdict(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();

    // baseline only: load the SEED + ONE telos-default materialize (no S6h mutation), exactly the
    // state run_eval.sh evaluates 04b against.
    let loaded = loader::load_seed(&pool, &load_seed_yaml()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();
    write::materialize_cogmap(&pool, loaded.cogmap, "telos-default", loaded.emitter)
        .await
        .unwrap();

    let all_pass: bool = sqlx::query_scalar(VERDICT_SQL)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        all_pass,
        "04b onboarding_s6_verdict all_pass must be true at baseline"
    );
}
