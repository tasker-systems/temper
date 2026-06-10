#![cfg(feature = "artifact-tests")]
//! Acceptance gate: the onboarding scenario as YAML roundtrips to the same regions + S6 verdicts as
//! run_eval.sh. Two checks, two states:
//!  - `passes_full_s6_runbook`: run the whole S6a–h runbook via run_scenario; its declarative asserts
//!    ARE the primary check (incl. S6b reproducibility, S6f plurality, S6h staleness/solo-joins-α).
//!  - `baseline_matches_04b_sql_verdict`: an INDEPENDENT encoding — the 04b verdict logic (a SQL
//!    aggregate over origin_uri) evaluated at the BASELINE state (load + one telos-default materialize,
//!    before the S6h mutation that retires solo's singleton). Same prose → same embeddings → same
//!    regions → same verdict.
//!  - `yaml_and_sql_seed_paths_produce_identical_region_membership`: the STRONG equivalence — diffs the
//!    actual region partition (canonical, UUID-independent) between the YAML path and the real
//!    `03_seed.sql` path. The verdict checks tolerate a wide band; this proves byte-equivalent regions.
//!
//! All reset the artifact and are serialized + ONNX-dependent.
mod common;

use temper_next::scenario::{bootseed, loader, model::Scenario, runner};
use temper_next::{embed, substrate, write};

const ONBOARDING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schema-artifact/scenarios/onboarding-cogmap.yaml"
);

fn load_yaml() -> Scenario {
    serde_yaml::from_str(&std::fs::read_to_string(ONBOARDING).unwrap()).unwrap()
}

#[tokio::test]
async fn passes_full_s6_runbook() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    runner::run_scenario(&pool, &load_yaml())
        .await
        .expect("declarative S6a-h asserts pass");

    // proof obligation 1: every fired event's payload deserializes into its typed struct
    temper_next::payloads::verify_ledger_roundtrip(&pool)
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

#[tokio::test]
async fn baseline_matches_04b_sql_verdict() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();

    // baseline only: load the substrate + ONE telos-default materialize (no S6h mutation), exactly the
    // state run_eval.sh evaluates 04b against.
    let loaded = loader::load_scenario(&pool, &load_yaml()).await.unwrap();
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

// Canonical, UUID-INDEPENDENT region partition signature for a cogmap at lens telos-default: each
// region's member origin_uris sorted within the region, regions sorted among themselves, then hashed.
// Independent of region UUIDs and group order, so it's comparable across seeding paths.
const PARTITION_SQL: &str = r#"
SELECT md5(string_agg(grp, '|' ORDER BY grp)) FROM (
  SELECT string_agg(res.origin_uri, ',' ORDER BY res.origin_uri) AS grp
  FROM kb_cogmap_region_members m
  JOIN kb_cogmap_regions r ON r.id = m.region_id AND NOT r.is_folded
  JOIN kb_cogmap_lenses  l ON l.id = r.lens_id AND l.name = 'telos-default'
  JOIN kb_resources    res ON res.id = m.member_id
  WHERE r.cogmap_id = $1
  GROUP BY m.region_id
) g
"#;

// The entity that seeded a cogmap (its genesis/steward emitter) — same honest source main.rs uses.
async fn genesis_emitter(pool: &sqlx::PgPool, cogmap: uuid::Uuid) -> uuid::Uuid {
    sqlx::query_scalar(
        "SELECT emitter_entity_id FROM kb_events \
         WHERE producing_anchor_table='kb_cogmaps' AND producing_anchor_id=$1 \
         ORDER BY occurred_at ASC, id ASC LIMIT 1",
    )
    .bind(cogmap)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn telos_default_partition(pool: &sqlx::PgPool, cogmap: uuid::Uuid) -> String {
    sqlx::query_scalar(PARTITION_SQL)
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn yaml_and_sql_seed_paths_produce_identical_region_membership() {
    // SQL-seed path: load 01+02+03_seed, materialize its onboarding-cogmap at the telos-default baseline.
    common::reset_artifact_with_seed();
    let pool = substrate::connect().await.unwrap();
    let sql_cogmap = substrate::cogmap_by_name(&pool, "onboarding-cogmap")
        .await
        .unwrap();
    let sql_emitter = genesis_emitter(&pool, sql_cogmap).await;
    embed::embed_chunks(&pool).await.unwrap();
    write::materialize_cogmap(&pool, sql_cogmap, "telos-default", sql_emitter)
        .await
        .unwrap();
    let sig_sql = telos_default_partition(&pool, sql_cogmap).await;

    // proof obligation 1 over the HAND-SQL seed path: 03_seed's payloads conform too
    temper_next::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("hand-SQL seed payload roundtrip");

    // YAML path: clean schema, boot-seed, load the YAML, materialize at the same baseline.
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_scenario(&pool, &load_yaml()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();
    write::materialize_cogmap(&pool, loaded.cogmap, "telos-default", loaded.emitter)
        .await
        .unwrap();
    let sig_yaml = telos_default_partition(&pool, loaded.cogmap).await;

    assert_eq!(
        sig_sql, sig_yaml,
        "YAML and SQL-seed paths must produce byte-identical region membership (keyed on origin_uri)"
    );
}
