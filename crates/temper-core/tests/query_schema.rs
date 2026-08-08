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

/// Names checked in this run, so the orphan guard can compare the two sets.
#[derive(Default)]
struct Checked(Vec<String>);

impl Checked {
    fn check<T: schemars::JsonSchema>(&mut self, name: &str) {
        self.0.push(name.to_string());
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
}

#[test]
fn query_contract_schemas_match_snapshots() {
    let mut c = Checked::default();
    c.check::<q::IdSet>("id_set");
    c.check::<q::IdKind>("id_kind");
    c.check::<q::IdProvenance>("id_provenance");
    c.check::<q::Extent>("extent");
    c.check::<q::BoundTerm>("bound_term");
    // EdgeKind is deliberately absent: it belongs to `types::graph`, is `sqlx::Type`-bound to the
    // DDL, and is snapshotted through `EdgeFilter` rather than as a query-owned type.
    c.check::<q::EdgeFilter>("edge_filter");
    c.check::<q::ResourceFilter>("resource_filter");
    c.check::<q::FilterField>("filter_field");
    // `bounds_mode` is gone: the relation is `StageRelation` now, and it lives inside `StageInput`
    // rather than beside it, so the input is snapshotted too — the nesting IS the contract change.
    c.check::<q::StageRelation>("stage_relation");
    c.check::<q::StageInput>("stage_input");
    c.check::<q::MetaDetail>("meta_detail");
    c.check::<q::StageDisposition>("disposition");
    c.check::<q::ActRefusal>("refusal");
    c.check::<q::PlanRefusal>("plan_refusal");
    // `refusal_disposition` is gone with `Composition.on_stage_refusal` — it described a case that
    // cannot occur. See the block where the field was, in `composition.rs`.
    c.check::<q::ActName>("act_name");
    c.check::<q::BuildState>("build_state");
    c.check::<q::Door>("door");
    c.check::<q::DoorReach>("door_reach");
    c.check::<q::QuantityScale>("quantity_scale");
    c.check::<q::ActQuantity>("act_quantity");
    c.check::<q::Disclosure>("disclosure");
    c.check::<q::ActDeclaration>("act_declaration");
    c.check::<q::ActInvocation>("act_invocation");
    c.check::<q::StageResult>("stage_result");
    c.check::<q::InputSource>("input_source");
    c.check::<q::MetaTruncated>("meta_truncated");
    c.check::<q::StageTrace>("stage_trace");
    c.check::<q::CompositionTrace>("composition_trace");
    c.check::<q::Intention>("intention");
    c.check::<q::ReturnSpec>("return_spec");
    c.check::<q::OutcomeDeclaration>("outcome_declaration");
    c.check::<q::Composition>("composition");

    no_orphaned_fixtures(&c);
}

/// Every committed fixture is claimed by a `check`.
///
/// `[added — 2026-08-08]` Removing a `check::<>` line leaves its snapshot on disk, where it reads
/// as coverage and asserts nothing — the same rot as a test no job enables, one layer down. This
/// run retired two (`bounds_mode`, `refusal_disposition`) and nothing would have noticed the files
/// staying behind.
///
/// **One direction only, and the other is deliberately not asserted here.** A `check` whose
/// fixture is absent is already caught by `check` itself: `unwrap_or_default()` compares the
/// rendered schema against `""` and the drift assertion fires. Adding a `missing` arm below looked
/// symmetric and was UNREACHABLE — `check` fails first without `UPDATE_SCHEMA`, and writes the
/// file with it. Observed by bite-probing both directions rather than reasoned about: direction
/// two failed on "schema drifted", never on the arm written for it. A dead assertion that reads as
/// coverage is the thing this function exists to prevent, so it is gone rather than kept for
/// symmetry.
fn no_orphaned_fixtures(checked: &Checked) {
    let on_disk: std::collections::BTreeSet<String> = std::fs::read_dir(DIR)
        .expect("fixture directory exists")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|f| f.strip_suffix(".schema.json").map(str::to_string))
        .collect();
    let claimed: std::collections::BTreeSet<String> = checked.0.iter().cloned().collect();

    assert_eq!(
        checked.0.len(),
        claimed.len(),
        "a name is checked twice; the second snapshot write silently wins over the first"
    );
    let orphaned: Vec<&String> = on_disk.difference(&claimed).collect();
    assert!(
        orphaned.is_empty(),
        "committed fixtures nothing checks — delete them: {orphaned:?}"
    );
}
