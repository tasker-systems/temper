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
    check::<p::BlobDeleted>("blob_deleted");
    check::<p::ResourceReblocked>("resource_reblocked");
    check::<p::BlobErased>("blob_erased");
    check::<p::PrincipalErased>("principal_erased");
    check::<p::PrincipalErasureRefused>("principal_erasure_refused");
    check::<p::ResourceErased>("resource_erased");
    check::<p::ResourceErasureRefused>("resource_erasure_refused");
    check::<p::BlockHistoryScrubbed>("block_history_scrubbed");
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

/// Stands in a migration's fixture list for a `$JS$` literal a LATER migration re-registered.
/// An applied migration is immutable, so its superseded literal can never match the live fixture
/// again; the entry keeps the positional pairing of the literals after it, and the superseding
/// migration's own entry pins the type.
const SUPERSEDED: &str = "(superseded by a later migration's entry)";

/// FAILS IF: a migration's embedded `$JS$` payload_schema literal drifts from the committed
/// fixture (review A-C1: `20260903000020_kb_blobs.sql` was pasted from a pre-`kb_blobs`-enum
/// render and nothing gated the seam). `payload_schemas_match_snapshots` pins Rust → fixture;
/// this pins fixture → migration, completing the repo == registry == Rust chain the migration
/// header declares. Structural (not byte) equality: the migration's own instruction says paste
/// byte for byte, but the load-bearing contract is the schema content — whitespace in a SQL
/// literal is not a wire fact.
#[test]
fn the_migration_literal_matches_the_committed_fixture() {
    // A migration may register SEVERAL typed events (the erasure vocabulary registers three);
    // its `$JS$` literals pair with this list IN ORDER — first literal to first fixture, etc.
    for (migration, fixtures) in [
        (
            "20260903000020_kb_blobs.sql",
            &["blob_committed.v1.schema.json"][..],
        ),
        (
            "20260909000010_resource_reblocked_dispositions.sql",
            &["resource_reblocked.v1.schema.json"],
        ),
        (
            "20260906000010_blob_strike_substrate.sql",
            &["blob_deleted.v1.schema.json"],
        ),
        (
            "20260909000015_erasure_act_vocabulary.sql",
            &[
                SUPERSEDED, // principal_erased: 20261002000020
                SUPERSEDED, // principal_erasure_refused: 20260930000050
                "blob_erased.v1.schema.json",
            ],
        ),
        (
            "20260929000010_resource_erasure_vocabulary.sql",
            &[
                SUPERSEDED, // resource_erased: 20261002000020
                SUPERSEDED, // resource_erasure_refused: 20260930000050
                SUPERSEDED, // block_history_scrubbed: 20261003000210
            ],
        ),
        (
            "20260930000050_erasure_present_truth_wording.sql",
            &[
                SUPERSEDED, // resource_erasure_refused: 20261003000210
                "principal_erasure_refused.v1.schema.json",
            ],
        ),
        (
            "20261002000020_erasure_payload_drops_propagated_to_clients.sql",
            &[
                "principal_erased.v1.schema.json",
                "resource_erased.v1.schema.json",
            ],
        ),
        (
            "20261003000210_block_history_scrub.sql",
            &[
                "block_history_scrubbed.v1.schema.json",
                "resource_erasure_refused.v1.schema.json",
            ],
        ),
    ] {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations/");
        let migration = std::fs::read_to_string(format!("{path}{migration}")).unwrap();
        let mut cursor = 0usize;
        for fixture_name in fixtures {
            let start =
                cursor + migration[cursor..].find("$JS$").unwrap_or_else(|| {
                    panic!("the migration carries a $JS$ payload_schema literal for {fixture_name}")
                }) + 4;
            let end = start
                + migration[start..]
                    .find("$JS$")
                    .expect("the $JS$ literal is closed");
            cursor = end + 4;
            if *fixture_name == SUPERSEDED {
                continue;
            }
            let literal: serde_json::Value = serde_json::from_str(&migration[start..end])
                .expect("the migration's embedded literal parses as JSON");
            let fixture: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(format!("{DIR}/{fixture_name}")).unwrap(),
            )
            .expect("the committed fixture parses as JSON");
            assert_eq!(
                literal, fixture,
                "the migration's embedded payload_schema drifted from the committed fixture \
                 ({fixture_name}) — re-paste the fixture into the $JS$ literal (see the \
                 migration's own instruction)"
            );
        }
    }
}
