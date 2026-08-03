#![cfg(feature = "mcp")]
//! Query-contract JSON-Schemas are emitted from the SAME structs the wire uses, so the artifact
//! and the code cannot drift. One committed snapshot per type.
//!
//! Regenerate: UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema
//!
//! PACKAGE-SCOPED AND FEATURE-PINNED ON PURPOSE. The emitted schema depends on feature
//! unification: with `mcp` on, the id newtypes emit INLINE (their `schemars(inline)` attribute);
//! under a different feature set they emit as `$ref`s into `$defs`. `mcp` is the authoritative
//! shape here because it is what an MCP tool schema actually carries. See the comment block at
//! tools/cargo-make/main.toml:91.

use temper_core::types::query as q;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/query");

fn check<T: schemars::JsonSchema>(name: &str) {
    let schema = schemars::SchemaGenerator::default().into_root_schema_for::<T>();
    let rendered = serde_json::to_string_pretty(&schema).unwrap() + "\n";
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
    check::<q::ActDeclaration>("act_declaration");
    check::<q::ActInvocation>("act_invocation");
    check::<q::ActResult>("act_result");
    check::<q::StageTrace>("stage_trace");
    check::<q::CompositionTrace>("composition_trace");
    check::<q::Intention>("intention");
    check::<q::OutcomeDeclaration>("outcome_declaration");
    check::<q::Composition>("composition");
}
