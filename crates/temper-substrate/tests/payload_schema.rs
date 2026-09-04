#![cfg(feature = "scenario-schema")]
//! Payload JSON-Schemas are emitted from the SAME structs `fire()` serializes — the wire contract
//! and the code can't drift. One committed snapshot per (type, version); the boot-seed stamps these
//! files into kb_event_types.payload_schema, so repo == registry == Rust types (spec §6 chain).
//! Regenerate: UPDATE_SCHEMA=1 cargo test -p temper-substrate --features scenario-schema --test payload_schema

use temper_substrate::payloads as p;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/payloads");

fn check<T: schemars::JsonSchema>(name: &str) {
    let schema = schemars::SchemaGenerator::default().into_root_schema_for::<T>();
    let rendered = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    let path = format!("{DIR}/{name}.v1.schema.json");
    if std::env::var("UPDATE_SCHEMA").is_ok() {
        std::fs::create_dir_all(DIR).unwrap();
        std::fs::write(&path, &rendered).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        rendered, committed,
        "{name} payload schema drifted — re-run with UPDATE_SCHEMA=1"
    );
}

#[test]
fn payload_schemas_match_snapshots() {
    check::<p::CogmapSeeded>("cogmap_seeded");
    check::<p::ResourceCreated>("resource_created");
    check::<p::RelationshipAsserted>("relationship_asserted");
    check::<p::PropertyAsserted>("property_asserted");
    check::<p::LensCreated>("lens_created");
    check::<p::RegionMaterialized>("region_materialized");
    check::<p::RelationshipRetyped>("relationship_retyped");
    check::<p::RelationshipReweighted>("relationship_reweighted");
    check::<p::RelationshipFolded>("relationship_folded");
    check::<p::RelationshipDecayed>("relationship_decayed");
    check::<p::RelationshipCorrected>("relationship_corrected");
    check::<p::BlockCreated>("block_created");
    check::<p::BlockMutated>("block_mutated");
    check::<p::BlockFolded>("block_folded");
    check::<p::BlockProvenanceCorrected>("block_provenance_corrected");
    check::<p::AdminLedgerOpened>("admin_ledger_opened");
    check::<p::GrantCreated>("grant_created");
    check::<p::GrantRevoked>("grant_revoked");
    check::<p::SlackPrincipalDisconnected>("slack_principal_disconnected");
    check::<p::PrincipalStandingChanged>("principal_standing_changed");
    check::<p::PrincipalGovernanceChanged>("principal_governance_changed");
    check::<p::SubscriptionDeliveryDisposed>("subscription_delivery_disposed");
    check::<p::DataArtifactCommitted>("data_artifact_committed");
    check::<p::ShapeDeclared>("shape_declared");
    check::<p::BlobCommitted>("blob_committed");
}

#[test]
fn snapshot_files_cover_exactly_the_typed_names() {
    let mut on_disk: Vec<String> = std::fs::read_dir(DIR)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|f| f.strip_suffix(".v1.schema.json").map(str::to_owned))
        .collect();
    on_disk.sort();
    let mut expected: Vec<String> = p::TYPED_EVENT_NAMES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(on_disk, expected);
}

/// FAILS IF: the migration's embedded `$JS$` payload_schema literal drifts from the committed
/// fixture (review A-C1: `20260903000020_kb_blobs.sql` was pasted from a pre-`kb_blobs`-enum
/// render and nothing gated the seam). `payload_schemas_match_snapshots` pins Rust → fixture;
/// this pins fixture → migration, completing the repo == registry == Rust chain the migration
/// header declares. Structural (not byte) equality: the migration's own instruction says paste
/// byte for byte, but the load-bearing contract is the schema content — whitespace in a SQL
/// literal is not a wire fact.
#[test]
fn the_migration_literal_matches_the_committed_fixture() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20260903000020_kb_blobs.sql"
    );
    let migration = std::fs::read_to_string(path).unwrap();
    let start = migration
        .find("$JS$")
        .expect("the migration carries a $JS$ payload_schema literal")
        + 4;
    let end = start
        + migration[start..]
            .find("$JS$")
            .expect("the $JS$ literal is closed");
    let literal: serde_json::Value = serde_json::from_str(&migration[start..end])
        .expect("the migration's embedded literal parses as JSON");
    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{DIR}/blob_committed.v1.schema.json")).unwrap(),
    )
    .expect("the committed fixture parses as JSON");
    assert_eq!(
        literal, fixture,
        "the migration's embedded payload_schema drifted from the committed fixture — \
         re-paste the fixture into the $JS$ literal (see the migration's own instruction)"
    );
}
