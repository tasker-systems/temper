#![cfg(feature = "mcp")]
//! Query-contract JSON-Schemas are emitted from the SAME structs the wire uses, so the artifact
//! and the code cannot drift. One committed snapshot per type.
//!
//! Regenerate: UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema
//!
//! FEATURE-PINNED ON PURPOSE. The emitted schema depends on feature unification: with `mcp` on,
//! the id newtypes emit INLINE (their `schemars(inline)` attribute); under a different feature set
//! they emit as `$ref`s into `$defs`. `mcp` is the authoritative shape here because it is what an
//! MCP tool schema actually carries. See the comment block at tools/cargo-make/main.toml:91.
//!
//! # Why the snapshot is canonicalized before comparison
//!
//! Object key order is NOT stable across cargo invocations, and the cause is remote from anything
//! this contract touches: `toon-format` / `serde_toon_format` (the `--format toon` CLI output
//! crates) enable `serde_json/preserve_order`. Under `--workspace` cargo unifies that feature into
//! temper-core's `serde_json`, which switches schemars' property map from `BTreeMap` (alphabetical)
//! to `IndexMap` (declaration order). Same types, same content, different byte order
//! `[verified — 2026-08-03]`.
//!
//! Left alone, this gate would PASS package-scoped and FAIL under `cargo make test`, which is how
//! it was first observed. The fix is to sort object keys before comparing, so the snapshot asserts
//! the schema's SHAPE and is invariant to which crates happen to share the build. Deliberately not
//! fixed by gating the test behind a feature `--workspace` leaves off: that would make it pass by
//! not running, and a gate that silently skips is worse than one that is merely order-sensitive.
//!
//! Arrays are left in place — `anyOf` branch order and `required` order are meaningful. Only
//! object keys are sorted, and object key order is never semantic in JSON Schema.

use temper_core::types::query as q;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/query");

/// Sort object keys recursively so the rendering does not depend on `serde_json/preserve_order`.
fn canonicalize(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                out.insert(k.clone(), canonicalize(&m[k]));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.iter().map(canonicalize).collect())
        }
        other => other.clone(),
    }
}

fn check<T: schemars::JsonSchema>(name: &str) {
    let schema = schemars::SchemaGenerator::default().into_root_schema_for::<T>();
    let value = canonicalize(&serde_json::to_value(&schema).unwrap());
    let rendered = serde_json::to_string_pretty(&value).unwrap() + "\n";
    let path = format!("{DIR}/{name}.schema.json");
    if std::env::var("UPDATE_SCHEMA").is_ok() {
        std::fs::create_dir_all(DIR).unwrap();
        std::fs::write(&path, &rendered).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        rendered, committed,
        "{name} query schema drifted — re-run with UPDATE_SCHEMA=1"
    );
}

#[test]
fn query_contract_schemas_match_snapshots() {
    check::<q::IdSet>("id_set");
    check::<q::IdKind>("id_kind");
    check::<q::IdProvenance>("id_provenance");
    check::<q::Extent>("extent");
    check::<q::BoundTerm>("bound_term");
    // EdgeKind is deliberately absent: it belongs to `types::graph`, is `sqlx::Type`-bound to the
    // DDL, and is snapshotted through `EdgeFilter` rather than as a query-owned type.
    check::<q::EdgeFilter>("edge_filter");
    check::<q::ResourceFilter>("resource_filter");
    check::<q::FilterField>("filter_field");
    check::<q::BoundsMode>("bounds_mode");
    check::<q::MetaDetail>("meta_detail");
    check::<q::StageDisposition>("disposition");
    check::<q::ActRefusal>("refusal");
    check::<q::RefusalDisposition>("refusal_disposition");
    check::<q::ActName>("act_name");
    check::<q::BuildState>("build_state");
    check::<q::Door>("door");
    check::<q::DoorReach>("door_reach");
    check::<q::QuantityScale>("quantity_scale");
    check::<q::ActQuantity>("act_quantity");
    check::<q::ActDeclaration>("act_declaration");
    check::<q::ActInvocation>("act_invocation");
    check::<q::ActResult>("act_result");
    check::<q::StageTrace>("stage_trace");
    check::<q::CompositionTrace>("composition_trace");
    check::<q::Intention>("intention");
    check::<q::OutcomeDeclaration>("outcome_declaration");
    check::<q::Composition>("composition");
}
