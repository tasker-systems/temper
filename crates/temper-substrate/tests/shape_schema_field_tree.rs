//! `shape_schema::compile` accepts only a field tree (spec
//! `2026-10-27-a-shape-is-a-field-tree-design`, §1 and §4): an allow-list of keywords in schema
//! positions, one pinned draft, and caps on depth, `enum` size, `pattern` size and `pattern` count.
//!
//! The allow-list is witnessed against the whole 2020-12 vocabulary in one table, each keyword its
//! own assertion: removing the walk makes every refused row compile. The data-position tests are
//! the opposite bite: walking `default` or `examples` — or a scalar `const` or `enum` member — as a
//! schema refuses them. Every cap is pinned on both sides, at the cap (compiles) and one past it
//! (refused).
//!
//! A `const` and every `enum` member is a scalar (spec
//! `2026-10-31-const-and-enum-members-are-scalars-design`, §1.1): an object, an array or a string
//! past `MAX_SHAPE_VALUE_STRING_BYTES` JSON-escaped bytes is refused wherever a schema can sit.
//!
//! A schema's parsed size has a budget (the same spec, §1.2): at most `MAX_SHAPE_NODES` JSON
//! values, at most `MAX_SHAPE_OBJECTS` of them objects, data positions included. Each cap is
//! witnessed alone, at both caps together, and against every admission profile it must fit.
//!
//! Plain `#[test]`s: needs no database, runs in the Unit CI job.

use serde_json::{json, Value};
use temper_substrate::shape_schema::{
    compile, count_up_to, first_failure, NodeCount, ShapeSchemaUncompilable, MAX_ENUM_ENTRIES,
    MAX_FAILURE_MESSAGE_BYTES, MAX_MIXED_ENUM_ENTRIES, MAX_PATTERN_SOURCE_BYTES,
    MAX_PROPERTY_NAME_BYTES, MAX_SHAPE_DEPTH, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS,
    MAX_SHAPE_PATTERNS, MAX_SHAPE_VALUE_STRING_BYTES,
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

// ── The allow-list, against the whole 2020-12 vocabulary ─────────────────────────────────────

/// Every keyword the field tree allows, with a value it may take. `$schema` and `$id` are
/// root-only, so they are witnessed at the root alone (`schema_and_id_compile_at_the_root`).
const ALLOWED: &[(&str, &str)] = &[
    ("type", r#""string""#),
    ("properties", r#"{ "a": { "type": "string" } }"#),
    ("required", r#"["a"]"#),
    ("additionalProperties", "false"),
    ("items", r#"{ "type": "string" }"#),
    ("enum", r#"["a", "b"]"#),
    ("const", r#""a""#),
    ("pattern", r#""^a""#),
    ("minLength", "1"),
    ("maxLength", "8"),
    ("minItems", "1"),
    ("maxItems", "8"),
    ("minimum", "0"),
    ("maximum", "8"),
    ("title", r#""t""#),
    ("description", r#""d""#),
    ("$comment", r#""c""#),
    ("default", r#""a""#),
    ("examples", r#"["a"]"#),
];

/// Every other keyword of the JSON Schema 2020-12 vocabularies — core, applicator, unevaluated,
/// validation, meta-data, format-annotation and content — and the earlier drafts' spellings of
/// what they replaced, each with a value it may take there, so that only the allow-list refuses it.
const REFUSED: &[(&str, &str)] = &[
    // core
    ("$ref", r##""#""##),
    ("$anchor", r#""a""#),
    ("$dynamicRef", r##""#""##),
    ("$dynamicAnchor", r#""a""#),
    ("$vocabulary", "{}"),
    ("$defs", r#"{ "x": { "type": "string" } }"#),
    // applicator
    ("prefixItems", r#"[{ "type": "string" }]"#),
    ("contains", r#"{ "type": "string" }"#),
    ("patternProperties", r#"{ "^a": { "type": "string" } }"#),
    ("dependentSchemas", r#"{ "a": { "required": ["b"] } }"#),
    ("propertyNames", r#"{ "maxLength": 8 }"#),
    ("if", r#"{ "type": "string" }"#),
    ("then", r#"{ "minLength": 1 }"#),
    ("else", r#"{ "type": "integer" }"#),
    ("allOf", r#"[{ "type": "object" }]"#),
    ("anyOf", r#"[{ "type": "string" }]"#),
    ("oneOf", r#"[{ "type": "string" }]"#),
    ("not", r#"{ "type": "string" }"#),
    // unevaluated
    ("unevaluatedItems", "false"),
    ("unevaluatedProperties", "false"),
    // validation
    ("multipleOf", "2"),
    ("exclusiveMaximum", "8"),
    ("exclusiveMinimum", "0"),
    ("uniqueItems", "true"),
    ("maxContains", "2"),
    ("minContains", "1"),
    ("maxProperties", "8"),
    ("minProperties", "1"),
    ("dependentRequired", r#"{ "a": ["b"] }"#),
    // meta-data
    ("deprecated", "true"),
    ("readOnly", "true"),
    ("writeOnly", "true"),
    // format-annotation
    ("format", r#""email""#),
    // content
    ("contentEncoding", r#""base64""#),
    ("contentMediaType", r#""text/plain""#),
    ("contentSchema", r#"{ "type": "string" }"#),
    // earlier drafts
    ("definitions", r#"{ "x": { "type": "string" } }"#),
    ("dependencies", r#"{ "a": ["b"] }"#),
    ("$recursiveRef", r##""#""##),
    ("$recursiveAnchor", "true"),
];

/// `keyword: value` as the one member of a schema at the root and in a `properties` value.
fn in_both_positions(keyword: &str, value: &str) -> [Value; 2] {
    let value: Value = serde_json::from_str(value).expect("a test value is JSON");
    [
        field_tree(json!({ keyword: value.clone() })),
        field_tree(json!({ "type": "object", "properties": { "a": { keyword: value } } })),
    ]
}

/// `member` (an object of schema members) as the root, as a `properties` value, as the `items`
/// schema and as an `additionalProperties` schema: the four positions the compile walks.
fn in_every_position(member: Value) -> [Value; 4] {
    [
        field_tree(member.clone()),
        field_tree(json!({ "type": "object", "properties": { "a": member.clone() } })),
        field_tree(json!({ "type": "array", "items": member.clone() })),
        field_tree(json!({ "type": "object", "additionalProperties": member })),
    ]
}

fn assert_refused_in_every_position(member: Value) {
    for schema in in_every_position(member) {
        assert_refused(&schema);
    }
}

fn assert_compiles_in_every_position(member: Value) {
    for schema in in_every_position(member) {
        assert_compiles(&schema);
    }
}

/// Every 2020-12 keyword off the allow-list is refused, at the root and nested. One table, so a
/// keyword cannot go unwitnessed: before it, `dependentSchemas` had a witness of its own while
/// `dependentRequired` and `dependencies` had none, all three refused by the walk's catch-all
/// arm. It also bites a keyword added to the allow-list without a decision: that keyword's row
/// here then compiles.
#[test]
fn every_keyword_off_the_allow_list_is_refused() {
    for (keyword, value) in REFUSED {
        for schema in in_both_positions(keyword, value) {
            assert_refused(&schema);
        }
    }
}

/// Every keyword on the allow-list compiles, at the root and nested — so a row of [`REFUSED`] that
/// fails is refused for its keyword, not for its form.
#[test]
fn every_keyword_on_the_allow_list_compiles() {
    for (keyword, value) in ALLOWED {
        for schema in in_both_positions(keyword, value) {
            assert_compiles(&schema);
        }
    }
}

/// The two lists are disjoint, so no keyword is both allowed and refused here.
#[test]
fn the_allowed_and_refused_lists_are_disjoint() {
    for (keyword, _) in ALLOWED {
        assert!(
            REFUSED.iter().all(|(refused, _)| refused != keyword),
            "{keyword} is in both lists"
        );
    }
}

/// The ask walk reads every construct a field tree may hold: a schema using every allowed keyword,
/// in every schema position the walk reads, passes `reads_whole` — so the walk does not read it as
/// declaring nothing — and its paths are declared. The walk keeps its own list, deliberately
/// apart from the compile's, so this is the test that bites when the compile's allow-list widens
/// without a decision on how the walk reads the new keyword.
#[test]
fn the_ask_walk_reads_whole_every_schema_built_of_allowed_keywords() {
    use temper_core::types::artifact_ask::{reads_whole, resolve, PathResolution, WalkBounds};
    let mut node = serde_json::Map::new();
    for (keyword, value) in ALLOWED {
        node.insert(
            keyword.to_string(),
            serde_json::from_str(value).expect("a test value is JSON"),
        );
    }
    let node = Value::Object(node);
    let mut schema = node.clone();
    schema["$id"] = json!("https://example.com/shape");
    schema["properties"]["a"] = node.clone();
    schema["properties"]["a"]["items"] = node;
    let schema = field_tree(schema);
    assert_compiles(&schema);
    assert!(reads_whole(&schema), "{schema}");
    for path in [
        vec!["a".to_string()],
        vec!["a".to_string(), "a".to_string()],
    ] {
        assert!(
            matches!(
                resolve(&schema, &path, &WalkBounds::BUILD),
                PathResolution::Declared(_)
            ),
            "{path:?}"
        );
    }
}

#[test]
fn schema_and_id_compile_at_the_root() {
    assert_compiles(&field_tree(json!({ "$id": "https://example.com/shape" })));
}

#[test]
fn refuses_an_unknown_key() {
    assert_refused(&field_tree(json!({ "x-custom": { "type": "string" } })));
}

/// 2020-12 meta-validation also refuses `items` as an array, so this is defence in depth with no
/// isolated bite on the walk.
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
/// keyword, a `$ref` or a custom `$schema` inside one is just a value. `default` and `examples` may
/// hold structured values; a `const` and an `enum` member are scalars, so the string that looks
/// like a keyword stands for the rest.
#[test]
fn data_positions_are_not_walked() {
    assert_compiles(&field_tree(json!({ "default": { "anyOf": [] } })));
    assert_compiles(&field_tree(json!({ "examples": [{ "$ref": "#" }] })));
    assert_compiles(&field_tree(json!({ "default": { "$schema": "urn:x" } })));
    assert_compiles(&field_tree(json!({ "default": { "$ref": "#" } })));
    assert_compiles(&field_tree(json!({ "examples": [{ "anyOf": [] }] })));
    assert_compiles(&field_tree(json!({ "const": "$ref" })));
    assert_compiles(&field_tree(json!({ "enum": ["$ref", "anyOf"] })));
}

// ── A const and an enum member are scalars ───────────────────────────────────────────────────

#[test]
fn an_object_const_is_refused() {
    assert_refused_in_every_position(json!({ "const": {} }));
    assert_refused_in_every_position(json!({ "const": { "type": "string" } }));
}

#[test]
fn an_array_const_is_refused() {
    assert_refused_in_every_position(json!({ "const": [] }));
}

#[test]
fn an_enum_with_an_object_member_is_refused() {
    assert_refused_in_every_position(json!({ "enum": ["a", {}] }));
}

#[test]
fn an_enum_with_an_array_member_is_refused() {
    assert_refused_in_every_position(json!({ "enum": ["a", []] }));
}

/// A string one byte past the cap, in a `const`, in an all-string enum and in a mixed enum.
#[test]
fn a_string_past_the_cap_is_refused() {
    let long = "a".repeat(MAX_SHAPE_VALUE_STRING_BYTES + 1);
    assert_refused_in_every_position(json!({ "const": long }));
    assert_refused_in_every_position(json!({ "enum": ["a", long] }));
    assert_refused_in_every_position(json!({ "enum": [1, long] }));
}

/// 250 quote characters are under the cap raw and twice that escaped, so a raw-length measure
/// admits them.
#[test]
fn a_string_under_the_cap_raw_but_over_it_escaped_is_refused() {
    let quotes = "\"".repeat(250);
    assert!(
        quotes.len() <= MAX_SHAPE_VALUE_STRING_BYTES,
        "under the cap raw"
    );
    let escaped = serde_json::to_string(&quotes).expect("a string serializes");
    assert!(
        escaped.len() - 2 > MAX_SHAPE_VALUE_STRING_BYTES,
        "over the cap escaped"
    );
    assert_refused_in_every_position(json!({ "const": quotes }));
    assert_refused_in_every_position(json!({ "enum": [quotes] }));
}

/// 129 two-byte characters are 258 bytes: past the cap in bytes, under it counted as characters.
#[test]
fn a_multibyte_string_past_the_cap_in_bytes_is_refused() {
    let accented = "é".repeat(129);
    assert!(accented.chars().count() <= MAX_SHAPE_VALUE_STRING_BYTES);
    assert!(accented.len() > MAX_SHAPE_VALUE_STRING_BYTES);
    assert_refused_in_every_position(json!({ "const": accented }));
    assert_refused_in_every_position(json!({ "enum": [accented] }));
}

/// A string exactly at the cap compiles as a `const`, in an all-string enum and in a mixed enum.
#[test]
fn a_string_at_the_cap_compiles() {
    let at_cap = "a".repeat(MAX_SHAPE_VALUE_STRING_BYTES);
    assert_compiles_in_every_position(json!({ "const": at_cap }));
    assert_compiles_in_every_position(json!({ "enum": ["a", at_cap] }));
    assert_compiles_in_every_position(json!({ "enum": [1, at_cap] }));
}

#[test]
fn every_scalar_kind_compiles_as_const_and_as_an_enum_member() {
    for scalar in [
        json!(null),
        json!(true),
        json!(false),
        json!(0),
        json!(-1.5),
        json!(""),
    ] {
        assert_compiles_in_every_position(json!({ "const": scalar }));
        assert_compiles_in_every_position(json!({ "enum": [scalar] }));
    }
}

/// A string is measured as `serde_json` writes it, whatever spelling the document gave it: a
/// control character written `\u0001` is six bytes escaped, one raw; a letter written `\u0041` is
/// one byte either way.
#[test]
fn a_string_written_with_unicode_escapes_is_measured_as_serde_escapes_it() {
    let schema_with = |escape: &str, count: usize| -> Value {
        let text = format!(r#"{{ "const": "{}" }}"#, escape.repeat(count));
        field_tree(serde_json::from_str(&text).expect("a test schema is JSON"))
    };
    let controls_in_cap = MAX_SHAPE_VALUE_STRING_BYTES / 6;
    assert_compiles(&schema_with(r"\u0001", controls_in_cap));
    assert_refused(&schema_with(r"\u0001", controls_in_cap + 1));
    assert_compiles(&schema_with(r"\u0041", MAX_SHAPE_VALUE_STRING_BYTES));
    assert_refused(&schema_with(r"\u0041", MAX_SHAPE_VALUE_STRING_BYTES + 1));
}

/// `serde_json` is built without `arbitrary_precision`, so a `Number` is fixed-size and a shape
/// value that is a number needs no length cap. `reference.rs` (decision P9) rests on the same
/// assumption. Without the feature a 400-digit integer literal is out of range and fails to parse
/// (at most 309 digits would parse as a lossy `f64`); with it, the literal parses as a
/// string-backed `Number` and this test goes red. The likeliest thing to turn it on is
/// `jsonschema`'s own `arbitrary-precision` feature.
#[test]
fn a_four_hundred_digit_number_does_not_parse() {
    assert!(
        serde_json::from_str::<Value>(&"9".repeat(400)).is_err(),
        "serde_json/arbitrary_precision is on: a number is no longer fixed-size"
    );
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
    let entries: Vec<String> = (0..MAX_ENUM_ENTRIES).map(|n| format!("v{n}")).collect();
    assert_compiles(&field_tree(json!({ "enum": entries })));
}

#[test]
fn enum_past_the_cap_is_refused() {
    let entries: Vec<String> = (0..=MAX_ENUM_ENTRIES).map(|n| format!("v{n}")).collect();
    assert_refused(&field_tree(json!({ "enum": entries })));
}

/// An enum with any non-string entry is held to the lower mixed cap: integers and strings, as
/// many as the cap, compile.
#[test]
fn mixed_enum_at_the_cap_compiles() {
    assert_compiles(&field_tree(
        json!({ "enum": mixed_enum(MAX_MIXED_ENUM_ENTRIES) }),
    ));
}

#[test]
fn mixed_enum_past_the_cap_is_refused() {
    assert_refused(&field_tree(
        json!({ "enum": mixed_enum(MAX_MIXED_ENUM_ENTRIES + 1) }),
    ));
}

/// `count` entries alternating integers and strings.
fn mixed_enum(count: usize) -> Vec<Value> {
    (0..count)
        .map(|n| {
            if n % 2 == 0 {
                json!(n)
            } else {
                json!(format!("v{n}"))
            }
        })
        .collect()
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

// ── The parsed-size budget ───────────────────────────────────────────────────────────────────
//
// These fixtures hold thousands of values, so a failure names the fixture and its count rather
// than printing the schema.

/// Every value and every object `schema` holds, counted with no cap to stop at.
fn whole_count(schema: &Value) -> NodeCount {
    count_up_to(schema, usize::MAX, usize::MAX)
}

/// `MAX_SHAPE_NODES + 1` scalars: wherever they sit, the document is past the node cap.
fn past_the_node_cap() -> Value {
    Value::Array(vec![json!(0); MAX_SHAPE_NODES + 1])
}

/// An object schema of `count` fields `f0000`, `f0001`, …, each `field`, with only `type` and
/// `properties` beside them: three values and two objects more than the fields hold.
fn object_of_fields(count: usize, field: &Value) -> Value {
    let properties: serde_json::Map<String, Value> = (0..count)
        .map(|index| (format!("f{index:04}"), field.clone()))
        .collect();
    json!({ "type": "object", "properties": properties })
}

/// One past the node cap is refused on its own, with the excess in `examples`, in `default` and
/// in an `additionalProperties` schema, each its own fixture. None holds more than two objects
/// (the root and the `additionalProperties` schema), so the objects cap is not what refuses it.
/// Bites dropping the node cap while the objects cap holds, and a count that skips a position.
#[test]
fn the_node_cap_alone_refuses() {
    let in_examples = field_tree(json!({ "examples": past_the_node_cap() }));
    let in_default = field_tree(json!({ "default": past_the_node_cap() }));
    let in_additional_properties = field_tree(json!({
        "type": "object",
        "additionalProperties": { "examples": past_the_node_cap() },
    }));
    for (placement, schema) in [
        ("examples", in_examples),
        ("default", in_default),
        ("an additionalProperties schema", in_additional_properties),
    ] {
        let count = whole_count(&schema);
        assert!(
            count.nodes > MAX_SHAPE_NODES && count.objects <= 2,
            "{placement}: {count:?}"
        );
        assert_eq!(
            compile(&schema).err(),
            Some(ShapeSchemaUncompilable),
            "{placement}: one past the node cap is refused"
        );
    }
}

/// One past the objects cap is refused on its own: `MAX_SHAPE_OBJECTS + 1` empty objects in
/// `examples`, with the root, `$schema` and the array beside them, far under the node cap. Bites
/// dropping the objects cap.
#[test]
fn the_objects_cap_alone_refuses() {
    let schema = field_tree(json!({
        "examples": Value::Array(vec![json!({}); MAX_SHAPE_OBJECTS + 1]),
    }));
    let count = whole_count(&schema);
    assert_eq!(
        (count.nodes, count.objects),
        (MAX_SHAPE_OBJECTS + 4, MAX_SHAPE_OBJECTS + 2),
        "{count:?}"
    );
    assert!(count.nodes < MAX_SHAPE_NODES, "{count:?}");
    assert_eq!(
        compile(&schema).err(),
        Some(ShapeSchemaUncompilable),
        "one past the objects cap is refused"
    );
}

/// A schema holding exactly `MAX_SHAPE_NODES` values, exactly `MAX_SHAPE_OBJECTS` of them
/// objects, compiles. Its `examples` hold the objects the root leaves to reach the objects cap,
/// then the scalars that reach the node cap. Bites an off-by-one on either cap.
#[test]
fn a_schema_at_both_caps_compiles() {
    // The root is the one object outside `examples`; the root, `$schema` and the `examples`
    // array are the three values outside its members.
    let objects = MAX_SHAPE_OBJECTS - 1;
    let scalars = MAX_SHAPE_NODES - 3 - objects;
    let mut examples = vec![json!({}); objects];
    examples.resize(objects + scalars, json!(0));
    let schema = field_tree(json!({ "examples": Value::Array(examples) }));
    assert_eq!(
        count_up_to(&schema, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS),
        NodeCount {
            nodes: MAX_SHAPE_NODES,
            objects: MAX_SHAPE_OBJECTS,
            over: false,
        }
    );
    assert!(compile(&schema).is_ok(), "a schema at both caps compiles");
}

/// Each synthetic admission profile in `MAX_SHAPE_NODES`'s doc compiles, and each fixture holds
/// exactly the values and objects its row states, so a fixture that drifts from its row fails
/// before it compiles. The table's first row, every live shape, is a production read with no
/// fixed counts, so it has no fixture here. Bites a cap set below an ordinary shape.
#[test]
fn every_admission_profile_compiles() {
    let described = json!({ "type": "string", "description": "A field." });
    let array_of_strings = json!({ "type": "array", "items": { "type": "string" } });
    let annotated = json!({
        "type": ["string", "null"],
        "title": "Field",
        "description": "A field.",
        "examples": ["an example"],
    });
    let numbers: Vec<usize> = (0..MAX_MIXED_ENUM_ENTRIES).collect();
    let numbered = json!({ "type": "integer", "enum": numbers });
    let codes: Vec<String> = (0..MAX_ENUM_ENTRIES)
        .map(|code| format!("c{code:03}"))
        .collect();
    let coded = json!({ "type": "string", "enum": codes });
    let profiles = [
        (
            "1,024 described strings",
            object_of_fields(1024, &described),
            3_075,
            1_026,
        ),
        (
            "1,024 arrays of strings",
            object_of_fields(1024, &array_of_strings),
            4_099,
            2_050,
        ),
        (
            "1,024 nullable strings with a title, a description and an example",
            object_of_fields(1024, &annotated),
            8_195,
            1_026,
        ),
        (
            "200 fields with 32-member enums",
            object_of_fields(200, &numbered),
            7_003,
            202,
        ),
        (
            "32 fields with 256-string enums",
            object_of_fields(32, &coded),
            8_291,
            34,
        ),
    ];
    for (name, schema, values, objects) in profiles {
        let count = whole_count(&schema);
        assert_eq!(
            (count.nodes, count.objects),
            (values, objects),
            "{name}: the fixture holds what its row states"
        );
        assert!(compile(&schema).is_ok(), "{name} compiles");
    }
}

/// Every `enum` member counts toward the budget, a repeated one each time it appears: 48 fields,
/// each listing the one string `"a"` `MAX_ENUM_ENTRIES` times, fit every per-keyword cap but not
/// the node budget. Bites a count of distinct values, which would hold this schema to 147.
#[test]
fn duplicate_enum_members_each_count_toward_the_budget() {
    let repeated = json!({ "enum": vec!["a"; MAX_ENUM_ENTRIES] });
    let schema = object_of_fields(48, &repeated);
    let count = whole_count(&schema);
    assert_eq!(
        (count.nodes, count.objects),
        (48 * (MAX_ENUM_ENTRIES + 2) + 3, 50),
        "{count:?}"
    );
    assert!(count.nodes > MAX_SHAPE_NODES, "{count:?}");
    assert_eq!(
        compile(&schema).err(),
        Some(ShapeSchemaUncompilable),
        "every repeated member counts"
    );
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

// ── The linear engine ────────────────────────────────────────────────────────────────────────
//
// `pattern` compiles on the `regex` crate, which has no lookaround and no backreferences: what
// `fancy-regex` accepted and could run in exponential time is refused at compile.

#[test]
fn refuses_a_lookahead_pattern() {
    assert_refused(&field_tree(
        json!({ "type": "string", "pattern": "^(?=a)a$" }),
    ));
}

#[test]
fn refuses_a_backreference_pattern() {
    assert_refused(&field_tree(
        json!({ "type": "string", "pattern": "^(a)\\1$" }),
    ));
}

// ── Stored-shape profiles ────────────────────────────────────────────────────────────────────
//
// One fixture per live shape, carrying that shape's keyword profile and depth only. Field names and
// patterns are invented; no stored schema is reproduced here.

/// The schema compiles, accepts `good` and refuses `bad`.
fn assert_verdicts(schema: &Value, good: &Value, bad: &Value) {
    let validator = compile(schema).unwrap_or_else(|_| panic!("{schema} must compile"));
    assert!(validator.is_valid(good), "{schema} must accept {good}");
    assert!(!validator.is_valid(bad), "{schema} must refuse {bad}");
}

/// Every stored shape's keyword profile compiles, and each gives a satisfied and an unsatisfied
/// verdict. `compilation-manifest` (two): root `$id`, `additionalProperties` as a boolean and as a
/// schema, `type` as an array, `items`, `minItems`, depth 3.
#[test]
fn stored_shape_profiles_compile() {
    // compilation-manifest, `additionalProperties` a boolean.
    let manifest_closed = json!({
        "$schema": DRAFT_2020_12,
        "$id": "urn:example:manifest-closed",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "entries": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "properties": { "label": { "type": ["string", "null"] } }
                }
            }
        }
    });
    assert_verdicts(
        &manifest_closed,
        &json!({ "entries": [{ "label": "alpha" }, { "label": null }] }),
        &json!({ "entries": [] }),
    );

    // compilation-manifest, `additionalProperties` a schema.
    let manifest_open = json!({
        "$schema": DRAFT_2020_12,
        "$id": "urn:example:manifest-open",
        "type": "object",
        "additionalProperties": { "type": ["string", "number"] },
        "properties": {
            "entries": {
                "type": "array",
                "minItems": 2,
                "items": {
                    "type": "object",
                    "properties": { "label": { "type": ["string", "null"] } }
                }
            }
        }
    });
    assert_verdicts(
        &manifest_open,
        &json!({ "entries": [{ "label": "a" }, { "label": "b" }], "note": "x" }),
        &json!({ "entries": [{ "label": "a" }] }),
    );

    // desktop-presented-view, three variants: `const`, one `pattern` of 8 bytes, string bounds,
    // depth 2.
    for (kind, pattern, good, bad) in [
        ("card", "^[a-z]+$", "abc", "ABC"),
        ("list", "^[0-9]+$", "123", "abc"),
        ("grid", "^[A-Z]+$", "ABC", "abc"),
    ] {
        assert!(
            pattern.len() <= 8,
            "the profile's pattern is 8 bytes or fewer"
        );
        let view = json!({
            "type": "object",
            "required": ["view"],
            "properties": {
                "view": {
                    "type": "object",
                    "properties": {
                        "kind": { "const": kind },
                        "slug": {
                            "type": "string",
                            "pattern": pattern,
                            "minLength": 2,
                            "maxLength": 12
                        }
                    }
                }
            }
        });
        assert_verdicts(
            &view,
            &json!({ "view": { "kind": kind, "slug": good } }),
            &json!({ "view": { "kind": kind, "slug": bad } }),
        );
    }

    // desktop-recent-work: `items`, `maxItems`, one `pattern` of about 60 bytes (a character class
    // with counted repetition), `minLength`, depth 3.
    let recent_pattern = "^[A-Za-z0-9_-]{1,32}/[A-Za-z0-9_.-]{1,24}(#[a-f0-9]{1,8})?$";
    assert!(
        (55..=65).contains(&recent_pattern.len()),
        "the profile's pattern is about 60 bytes"
    );
    let recent = json!({
        "type": "object",
        "properties": {
            "recent": {
                "type": "array",
                "maxItems": 3,
                "items": {
                    "type": "object",
                    "properties": {
                        "ref": { "type": "string", "minLength": 3, "pattern": recent_pattern }
                    }
                }
            }
        }
    });
    assert_verdicts(
        &recent,
        &json!({ "recent": [{ "ref": "team-a/notes_1#ab12" }] }),
        &json!({ "recent": [{ "ref": "!!" }] }),
    );

    // scratch-measurement: `type`, `properties`, `required`, depth 1.
    let measurement = json!({
        "type": "object",
        "required": ["reading"],
        "properties": { "reading": { "type": "number" } }
    });
    assert_verdicts(
        &measurement,
        &json!({ "reading": 1.5 }),
        &json!({ "reading": "high" }),
    );
}

// ── one failure, cut to a bound ──────────────────────────────────────────────────────────────

fn three_short_strings() -> Value {
    field_tree(json!({
        "type": "object",
        "properties": {
            "alpha": { "type": "string", "minLength": 5 },
            "bravo": { "type": "string", "minLength": 5 },
            "charlie": { "type": "string", "minLength": 5 },
        },
        "required": ["alpha", "bravo", "charlie"],
    }))
}

#[test]
fn first_failure_names_one_of_three() {
    let validator = compile(&three_short_strings()).expect("a field tree compiles");
    let content = json!({ "alpha": "a", "bravo": "b", "charlie": "c" });
    let (path, _message) = first_failure(&validator, &content).expect("three violations");
    let named = ["alpha", "bravo", "charlie"]
        .iter()
        .filter(|name| path.contains(*name))
        .count();
    assert_eq!(named, 1, "exactly one violating path is named, got: {path}");
}

#[test]
fn first_failure_is_none_when_satisfied() {
    let validator = compile(&three_short_strings()).expect("a field tree compiles");
    let content = json!({ "alpha": "aaaaa", "bravo": "bbbbb", "charlie": "ccccc" });
    assert_eq!(first_failure(&validator, &content), None);
}

/// `maxLength` renders the failing instance into its message (`"<instance>" is longer than 1
/// character`), so a long instance makes a long message. (`const` does not echo it.)
#[test]
fn a_long_failure_message_is_cut_to_the_bound() {
    let validator = compile(&field_tree(json!({ "type": "string", "maxLength": 1 })))
        .expect("a field tree compiles");
    let (_path, message) =
        first_failure(&validator, &json!("y".repeat(10 * 1024))).expect("a violation");
    assert!(
        message.len() <= MAX_FAILURE_MESSAGE_BYTES,
        "the message is {} bytes",
        message.len()
    );
    assert!(message.ends_with('…'), "a cut message ends in an ellipsis");
}

/// The cut lands on a character boundary: the instance is `x` then 4,000 two-byte characters, so
/// in the uncut message (which opens with a `"`) the characters start at even offsets and the cut
/// point `MAX_FAILURE_MESSAGE_BYTES - 3` (1021) falls inside one. Slicing there would panic, and the
/// walk back to a boundary is what the test exercises.
#[test]
fn the_cut_never_splits_a_character() {
    let validator = compile(&field_tree(json!({ "type": "string", "maxLength": 1 })))
        .expect("a field tree compiles");
    let content = json!(format!("x{}", "é".repeat(4000)));
    let uncut = jsonschema::validate(
        &field_tree(json!({ "type": "string", "maxLength": 1 })),
        &content,
    )
    .expect_err("a violation")
    .to_string();
    assert!(
        !uncut.is_char_boundary(MAX_FAILURE_MESSAGE_BYTES - 3),
        "the fixture's cut point is inside a character"
    );
    let (_path, message) = first_failure(&validator, &content).expect("a violation");
    assert!(message.len() <= MAX_FAILURE_MESSAGE_BYTES);
    assert!(message.ends_with('…'));
    let prefix = message.strip_suffix('…').expect("ends with an ellipsis");
    assert!(uncut.starts_with(prefix));
}

// ── Property names ───────────────────────────────────────────────────────────────────────────

#[test]
fn a_property_name_at_the_cap_compiles() {
    let name = "k".repeat(MAX_PROPERTY_NAME_BYTES);
    assert_compiles(&field_tree(
        json!({ "properties": { name.clone(): { "type": "string" } }, "required": [name] }),
    ));
}

#[test]
fn a_property_name_past_the_cap_is_refused() {
    let name = "k".repeat(MAX_PROPERTY_NAME_BYTES + 1);
    assert_refused(&field_tree(
        json!({ "properties": { name: { "type": "string" } } }),
    ));
}

#[test]
fn a_required_entry_past_the_cap_is_refused() {
    let name = "k".repeat(MAX_PROPERTY_NAME_BYTES + 1);
    assert_refused(&field_tree(json!({ "required": [name] })));
}

/// The refusal names the one `$schema` a field tree may carry, and that a trailing `#` is allowed,
/// so a declarer refused for an `http://` or draft-07 spelling is told what to write instead.
#[test]
fn the_refusal_names_the_accepted_schema_spellings() {
    let sentence = ShapeSchemaUncompilable.to_string();
    assert!(sentence.contains(DRAFT_2020_12), "{sentence}");
    assert!(sentence.contains("trailing #"), "{sentence}");
}

/// The refusal states the value rule, so a declarer refused for a structured `const` or a long
/// string is told what a value may be.
#[test]
fn the_refusal_names_the_value_rule() {
    let sentence = ShapeSchemaUncompilable.to_string();
    let clause = format!(
        "const and enum members that are strings (at most {MAX_SHAPE_VALUE_STRING_BYTES} bytes, \
         JSON-escaped), numbers, booleans or null"
    );
    assert!(sentence.contains(&clause), "{sentence}");
}

/// The refusal states the node budget, and that it can refuse what every other cap allows, so a
/// declarer refused for a schema too large once parsed is told the bound it passed.
#[test]
fn the_refusal_names_the_budget() {
    let sentence = ShapeSchemaUncompilable.to_string();
    let clause = format!(
        "holding at most {MAX_SHAPE_NODES} JSON values in all, at most {MAX_SHAPE_OBJECTS} of \
         them objects (a budget that can refuse a schema every other cap allows)"
    );
    assert!(sentence.contains(&clause), "{sentence}");
}
