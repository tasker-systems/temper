#![cfg(feature = "scenario-schema")]
//! The scenario JSON Schema is emitted from the SAME structs the loader reads — so the wire shape and
//! the config parser can't drift. This snapshot test fails if the derived schema changes; regenerate
//! the committed snapshot with `UPDATE_SCHEMA=1 cargo test -p temper-next --features scenario-schema`.

use temper_next::scenario::model::Scenario;

const SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schema-artifact/scenarios/scenario.schema.json"
);

#[test]
fn scenario_json_schema_matches_snapshot() {
    let schema = schemars::schema_for!(Scenario);
    let rendered = serde_json::to_string_pretty(&schema).unwrap() + "\n";

    if std::env::var("UPDATE_SCHEMA").is_ok() {
        std::fs::write(SNAPSHOT, &rendered).unwrap();
    }
    let committed = std::fs::read_to_string(SNAPSHOT).unwrap_or_default();
    assert_eq!(
        rendered, committed,
        "scenario schema drifted — re-run with UPDATE_SCHEMA=1 to refresh the snapshot"
    );
}
