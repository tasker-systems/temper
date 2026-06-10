//! Shared setup for the scenario write-path artifact tests.
//!
//! These tests OWN the `temper_next` namespace: each resets it to a clean `01_schema` + `02_functions`
//! (no `03_seed.sql`), then boot-seeds + loads its own scenario. `01_schema.sql` drops and recreates
//! the `temper_next` schema, so loading 01+02 is a full reset. The tests are serialized via the
//! `temper-next-write` nextest test-group so resets never race a sibling's queries.
//!
//! (The legacy read-path tests — materialize/substrate_read/embed_job — instead assume `03_seed.sql`
//! is loaded externally, so the two suites are run separately. M2 retires the legacy path.)

#![allow(dead_code)]

/// Drop + reload the artifact schema and functions, leaving a clean (un-seeded) `temper_next`.
pub fn reset_artifact() {
    load_files(&["01_schema", "02_functions"]);
}

/// Like [`reset_artifact`] but also loads the hand-written `03_seed.sql` (the legacy SQL-seed path) —
/// used by the cross-path equivalence test to materialize the SQL-seeded onboarding-cogmap.
pub fn reset_artifact_with_seed() {
    load_files(&["01_schema", "02_functions", "03_seed"]);
}

fn load_files(files: &[&str]) {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for artifact tests");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    for f in files {
        let path = format!("{root}/schema-artifact/{f}.sql");
        let status = std::process::Command::new("psql")
            .args([url.as_str(), "-q", "-v", "ON_ERROR_STOP=1", "-f", &path])
            .status()
            .expect("failed to run psql (is it on PATH?)");
        assert!(status.success(), "psql -f {f}.sql failed during reset");
    }
}

/// Canonical, UUID-INDEPENDENT region partition signature for a cogmap at lens `telos-default`:
/// each region's member origin_uris sorted within the region, regions sorted among themselves, then
/// hashed. Independent of region/member UUIDs and group order, so it is comparable across seeding
/// paths and across separate instantiations of the same seed (identity-as-input regenerates UUIDs
/// per load, so `membership_fingerprint` is only comparable within one instantiation).
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

pub async fn telos_default_partition(pool: &sqlx::PgPool, cogmap: uuid::Uuid) -> String {
    sqlx::query_scalar(PARTITION_SQL)
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
}
