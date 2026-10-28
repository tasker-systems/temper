//! `shape_schema::compile` accepts only a field tree (spec
//! `2026-10-27-a-shape-is-a-field-tree-design`, §1 and §4): an allow-list of keywords in schema
//! positions, one pinned draft, and caps on depth, `enum` size, `pattern` size and `pattern` count.
//!
//! Each refused keyword family is its own witness, so that each is falsified on its own: removing
//! the walk makes every refusal test here compile. The data-position tests are the opposite bite:
//! walking `const`, `enum`, `default` or `examples` as schemas refuses them. Every cap is pinned on
//! both sides, at the cap (compiles) and one past it (refused).
//!
//! Plain `#[test]`s: needs no database, runs in the Unit CI job.

use serde_json::{json, Value};
use temper_substrate::shape_schema::{
    compile, ShapeSchemaUncompilable, MAX_ENUM_ENTRIES, MAX_PATTERN_SOURCE_BYTES, MAX_SHAPE_DEPTH,
    MAX_SHAPE_PATTERNS,
};

const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// A schema on the pinned draft with the given members beside `$schema`.
fn field_tree(members: Value) -> Value {
    let mut schema = json!({ "$schema": DRAFT_2020_12 });
    let Value::Object(extra) = members else {
        panic!("a test schema's members are an object");
    };
    schema.as_object_mut().expect("an object").extend(extra);
    schema
}

fn assert_refused(schema: &Value) {
    assert_eq!(
        compile(schema).err(),
        Some(ShapeSchemaUncompilable),
        "schema {schema} must be refused with the fixed error"
    );
}

fn assert_compiles(schema: &Value) {
    assert!(
        compile(schema).is_ok(),
        "schema {schema} must compile as a field tree"
    );
}

/// One step into a nested schema position.
#[derive(Clone, Copy)]
enum Step {
    Property,
    Items,
    AdditionalProperties,
}

/// A schema nested through `steps`, outermost first, ending in a string leaf. The root is depth 0,
/// so `steps.len()` is the leaf's depth.
fn chain(steps: &[Step]) -> Value {
    let mut node = json!({ "type": "string" });
    for step in steps.iter().rev() {
        node = match step {
            Step::Property => json!({ "type": "object", "properties": { "a": node } }),
            Step::Items => json!({ "type": "array", "items": node }),
            Step::AdditionalProperties => {
                json!({ "type": "object", "additionalProperties": node })
            }
        };
    }
    let mut schema = json!({ "$schema": DRAFT_2020_12 });
    schema
        .as_object_mut()
        .expect("an object")
        .extend(node.as_object().expect("an object").clone());
    schema
}

// ── Refused keyword families, one witness each ───────────────────────────────────────────────

#[test]
fn refuses_ref() {
    assert_refused(&field_tree(
        json!({ "properties": { "a": { "$ref": "#/properties/b" } } }),
    ));
}

#[test]
fn refuses_defs() {
    assert_refused(&field_tree(
        json!({ "$defs": { "x": { "type": "string" } } }),
    ));
}

#[test]
fn refuses_all_of() {
    assert_refused(&field_tree(json!({ "allOf": [{ "type": "object" }] })));
}

#[test]
fn refuses_any_of() {
    assert_refused(&field_tree(json!({ "anyOf": [{ "type": "string" }] })));
}

#[test]
fn refuses_one_of() {
    assert_refused(&field_tree(json!({ "oneOf": [{ "type": "string" }] })));
}

#[test]
fn refuses_not() {
    assert_refused(&field_tree(json!({ "not": { "type": "string" } })));
}

#[test]
fn refuses_if_then_else() {
    assert_refused(&field_tree(json!({
        "if": { "type": "string" },
        "then": { "minLength": 1 },
        "else": { "type": "integer" },
    })));
}

#[test]
fn refuses_pattern_properties() {
    assert_refused(&field_tree(
        json!({ "patternProperties": { "^a": { "type": "string" } } }),
    ));
}

#[test]
fn refuses_unique_items() {
    assert_refused(&field_tree(json!({ "type": "array", "uniqueItems": true })));
}

#[test]
fn refuses_format() {
    assert_refused(&field_tree(json!({ "type": "string", "format": "email" })));
}

#[test]
fn refuses_prefix_items() {
    assert_refused(&field_tree(
        json!({ "prefixItems": [{ "type": "string" }] }),
    ));
}

#[test]
fn refuses_contains() {
    assert_refused(&field_tree(
        json!({ "type": "array", "contains": { "type": "string" } }),
    ));
}

#[test]
fn refuses_dependent_schemas() {
    assert_refused(&field_tree(json!({
        "dependentSchemas": { "a": { "required": ["b"] } },
    })));
}

#[test]
fn refuses_unevaluated_properties() {
    assert_refused(&field_tree(json!({ "unevaluatedProperties": false })));
}

#[test]
fn refuses_an_unknown_key() {
    assert_refused(&field_tree(json!({ "x-custom": { "type": "string" } })));
}

#[test]
fn refuses_items_as_an_array() {
    assert_refused(&field_tree(
        json!({ "type": "array", "items": [{ "type": "string" }] }),
    ));
}

#[test]
fn refuses_id_below_the_root() {
    assert_refused(&field_tree(json!({
        "properties": { "a": { "$id": "urn:example:a", "type": "string" } },
    })));
}

#[test]
fn refuses_schema_below_the_root() {
    assert_refused(&field_tree(json!({
        "properties": { "a": { "$schema": DRAFT_2020_12, "type": "string" } },
    })));
}

#[test]
fn refuses_an_http_schema_spelling() {
    assert_refused(&json!({
        "$schema": "http://json-schema.org/draft/2020-12/schema",
        "type": "object",
    }));
}

#[test]
fn refuses_a_versionless_schema_spelling() {
    assert_refused(&json!({ "$schema": "https://json-schema.org/schema", "type": "object" }));
}

#[test]
fn refuses_draft_07() {
    assert_refused(&json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
    }));
}

// ── What compiles ────────────────────────────────────────────────────────────────────────────

#[test]
fn accepts_the_schema_uri_with_a_trailing_hash() {
    assert_compiles(&json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema#",
        "type": "object",
    }));
}

#[test]
fn accepts_an_absent_schema_uri() {
    assert_compiles(&json!({ "type": "object", "properties": { "a": { "type": "string" } } }));
}

/// `const`, `enum`, `default` and `examples` hold data: nothing in them is read as a schema, so a
/// keyword, a `$ref` or a custom `$schema` inside one is just a value.
#[test]
fn data_positions_are_not_walked() {
    assert_compiles(&field_tree(json!({ "const": { "anyOf": [] } })));
    assert_compiles(&field_tree(json!({ "enum": [{ "$ref": "#" }] })));
    assert_compiles(&field_tree(json!({ "const": { "$schema": "urn:x" } })));
}

#[test]
fn boolean_subschemas_compile() {
    assert_compiles(&field_tree(json!({
        "type": "object",
        "properties": { "a": false },
    })));
    assert_compiles(&field_tree(json!({ "type": "array", "items": true })));
}

// ── Depth ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn depth_at_the_cap_compiles() {
    assert_compiles(&chain(&[Step::Property; MAX_SHAPE_DEPTH]));
}

#[test]
fn depth_past_the_cap_is_refused() {
    assert_refused(&chain(&[Step::Property; MAX_SHAPE_DEPTH + 1]));
}

#[test]
fn depth_counts_items_and_additional_properties() {
    let steps: Vec<Step> = [Step::Property, Step::Items, Step::AdditionalProperties]
        .into_iter()
        .cycle()
        .take(MAX_SHAPE_DEPTH + 1)
        .collect();
    assert_refused(&chain(&steps));
    assert_compiles(&chain(&steps[..MAX_SHAPE_DEPTH]));
}

// ── Caps ─────────────────────────────────────────────────────────────────────────────────────

#[test]
fn enum_at_the_cap_compiles() {
    let entries: Vec<usize> = (0..MAX_ENUM_ENTRIES).collect();
    assert_compiles(&field_tree(json!({ "enum": entries })));
}

#[test]
fn enum_past_the_cap_is_refused() {
    let entries: Vec<usize> = (0..=MAX_ENUM_ENTRIES).collect();
    assert_refused(&field_tree(json!({ "enum": entries })));
}

#[test]
fn pattern_source_at_the_cap_compiles() {
    assert_compiles(&field_tree(json!({
        "type": "string",
        "pattern": "a".repeat(MAX_PATTERN_SOURCE_BYTES),
    })));
}

#[test]
fn pattern_source_past_the_cap_is_refused() {
    assert_refused(&field_tree(json!({
        "type": "string",
        "pattern": "a".repeat(MAX_PATTERN_SOURCE_BYTES + 1),
    })));
}

/// A shape with `count` properties, each carrying a distinct `^a{N}$` pattern.
fn shape_with_patterns(count: usize) -> Value {
    let properties: serde_json::Map<String, Value> = (1..=count)
        .map(|n| {
            (
                format!("p{n}"),
                json!({ "type": "string", "pattern": format!("^a{{{n}}}$") }),
            )
        })
        .collect();
    field_tree(json!({ "type": "object", "properties": properties }))
}

#[test]
fn pattern_count_at_the_cap_compiles() {
    assert_compiles(&shape_with_patterns(MAX_SHAPE_PATTERNS));
}

#[test]
fn pattern_count_past_the_cap_is_refused() {
    assert_refused(&shape_with_patterns(MAX_SHAPE_PATTERNS + 1));
}

/// `count` properties whose patterns sit one level down: odd ones under an `items` schema, even
/// ones under an `additionalProperties` schema.
fn shape_with_nested_patterns(count: usize) -> Value {
    let properties: serde_json::Map<String, Value> = (1..=count)
        .map(|n| {
            let leaf = json!({ "type": "string", "pattern": format!("^a{{{n}}}$") });
            let holder = if n % 2 == 0 {
                json!({ "type": "object", "additionalProperties": leaf })
            } else {
                json!({ "type": "array", "items": leaf })
            };
            (format!("p{n}"), holder)
        })
        .collect();
    field_tree(json!({ "type": "object", "properties": properties }))
}

#[test]
fn patterns_under_items_and_additional_properties_count() {
    assert_compiles(&shape_with_nested_patterns(MAX_SHAPE_PATTERNS));
    assert_refused(&shape_with_nested_patterns(MAX_SHAPE_PATTERNS + 1));
}

/// Properties merely NAMED for keywords (`pattern`, `$id`, `$schema`, `enum`) are names, not
/// keywords: none is counted, refused or read as one. With the cap's worth of real patterns
/// elsewhere the shape still compiles, which a count that included the property named `pattern`
/// would exceed.
#[test]
fn keyword_named_properties_are_names() {
    let mut schema = shape_with_patterns(MAX_SHAPE_PATTERNS);
    let properties = schema["properties"].as_object_mut().expect("properties");
    properties.insert("pattern".to_owned(), json!({ "type": "string" }));
    properties.insert("$ref".to_owned(), json!({ "type": "string" }));
    properties.insert("type".to_owned(), json!({ "type": "string" }));
    properties.insert("$id".to_owned(), json!({ "type": "string" }));
    properties.insert("$schema".to_owned(), json!({ "type": "string" }));
    properties.insert("enum".to_owned(), json!({ "type": "string" }));
    assert_compiles(&schema);
}

// ── Forms the walk leaves to meta-validation, and one it fixes ───────────────────────────────

#[test]
fn malformed_forms_answer_the_fixed_error() {
    assert_refused(&field_tree(json!({ "type": ["string", "string"] })));
    assert_refused(&field_tree(json!({ "required": [1] })));
}

#[test]
fn enum_not_an_array_is_refused() {
    assert_refused(&field_tree(json!({ "enum": 5 })));
}
