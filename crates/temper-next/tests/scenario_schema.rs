#![cfg(feature = "scenario-schema")]
//! The seed + scenario JSON Schemas are emitted from the SAME structs the loader reads — so the wire
//! shapes and the config parser can't drift. These snapshot tests fail if a derived schema changes;
//! regenerate the committed snapshots with
//! `UPDATE_SCHEMA=1 cargo test -p temper-next --features scenario-schema`.

use temper_next::scenario::model::{Scenario, Seed};

const SCENARIO_SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/scenarios/scenario.schema.json"
);
const SEED_SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/seed.schema.json"
);
const ACCESS_SCENARIO_SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/access-scenarios/access-scenario.schema.json"
);

fn assert_snapshot(rendered: &str, snapshot: &str, kind: &str) {
    if std::env::var("UPDATE_SCHEMA").is_ok() {
        std::fs::write(snapshot, rendered).unwrap();
    }
    let committed = std::fs::read_to_string(snapshot).unwrap_or_default();
    assert_eq!(
        rendered, committed,
        "{kind} schema drifted — re-run with UPDATE_SCHEMA=1 to refresh the snapshot"
    );
}

#[test]
fn scenario_json_schema_matches_snapshot() {
    let schema = schemars::schema_for!(Scenario);
    let rendered = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    assert_snapshot(&rendered, SCENARIO_SNAPSHOT, "scenario");
}

#[test]
fn seed_json_schema_matches_snapshot() {
    let schema = schemars::schema_for!(Seed);
    let rendered = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    assert_snapshot(&rendered, SEED_SNAPSHOT, "seed");
}

#[test]
fn access_scenario_json_schema_matches_snapshot() {
    let schema = schemars::schema_for!(temper_next::scenario::access::model::AccessScenario);
    let rendered = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    assert_snapshot(&rendered, ACCESS_SCENARIO_SNAPSHOT, "access-scenario");
}
