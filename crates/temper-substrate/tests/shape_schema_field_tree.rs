//! `shape_schema::compile` accepts only a field tree (spec
//! `2026-10-27-a-shape-is-a-field-tree-design`, §1 and §4): an allow-list of keywords in schema
//! positions, one pinned draft, and caps on depth, `enum` size, `pattern` size and `pattern` count.
//!
//! The allow-list is witnessed against the whole 2020-12 vocabulary in one table, each keyword its
//! own assertion: removing the walk makes every refused row compile. The data-position tests are
//! the opposite bite: walking `const`, `enum`, `default` or `examples` as schemas refuses them.
//! Every cap is pinned on both sides, at the cap (compiles) and one past it (refused).
//!
//! Plain `#[test]`s: needs no database, runs in the Unit CI job.

use serde_json::{json, Value};
use temper_substrate::shape_schema::{
    compile, first_failure, ShapeSchemaUncompilable, MAX_ENUM_ENTRIES, MAX_FAILURE_MESSAGE_BYTES,
    MAX_MIXED_ENUM_ENTRIES, MAX_PATTERN_SOURCE_BYTES, MAX_PROPERTY_NAME_BYTES, MAX_SHAPE_DEPTH,
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
    refusal(schema);
}

/// What `schema`'s refusal names, asserting it is refused with the fixed sentence as its `Display`
/// and a named refusal within the bound beside it.
fn refusal(schema: &Value) -> String {
    let error = compile(schema).expect_err("the schema must be refused");
    assert_eq!(
        error.to_string(),
        ShapeSchemaUncompilable::default().to_string(),
        "schema {schema} must be refused with the fixed sentence as its Display"
    );
    let detailed = error.detailed();
    assert!(
        detailed.len() <= MAX_FAILURE_MESSAGE_BYTES,
        "{} bytes: {detailed}",
        detailed.len()
    );
    error
        .refused()
        .unwrap_or_else(|| panic!("schema {schema} must name what was refused"))
        .to_owned()
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
    // An annotation, never an assertion (shape ruling R1).
    ("format", r#""date-time""#),
    // Carried and ignored, whatever it holds (shape ruling R2).
    (
        "x-ui",
        r##"{ "$ref": "#", "allOf": [7], "properties": 7 }"##,
    ),
];

/// Every other keyword of the JSON Schema 2020-12 vocabularies — core, applicator, unevaluated,
/// validation, meta-data and content — and the earlier drafts' spellings of
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
    assert_refused(&field_tree(json!({ "custom": { "type": "string" } })));
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
/// keyword, a `$ref` or a custom `$schema` inside one is just a value.
#[test]
fn data_positions_are_not_walked() {
    assert_compiles(&field_tree(json!({ "const": { "anyOf": [] } })));
    assert_compiles(&field_tree(json!({ "enum": [{ "$ref": "#" }] })));
    assert_compiles(&field_tree(json!({ "const": { "$schema": "urn:x" } })));
    assert_compiles(&field_tree(json!({ "default": { "$ref": "#" } })));
    assert_compiles(&field_tree(json!({ "examples": [{ "anyOf": [] }] })));
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
    let sentence = ShapeSchemaUncompilable::default().to_string();
    assert!(sentence.contains(DRAFT_2020_12), "{sentence}");
    assert!(sentence.contains("trailing #"), "{sentence}");
}

// ── format and x-* are inert annotations (shape rulings R1, R2) ──────────────────────────────

/// `format` asserts nothing: the compile turns on no format validation, so a string that is not a
/// date-time satisfies a `format: date-time` field. Witnessed against the `jsonschema` in
/// `Cargo.lock`, so an upgrade that turned format assertion on by default bites here.
#[test]
fn format_is_an_annotation_and_asserts_nothing() {
    let schema = field_tree(json!({
        "type": "object",
        "properties": {
            "when": { "type": "string", "format": "date-time" },
            "mail": { "type": "string", "format": "email" },
            "id": { "type": "string", "format": "uuid" },
        }
    }));
    let validator = compile(&schema).expect("format is an annotation");
    let content = json!({ "when": "not a date", "mail": "not an address", "id": "nope" });
    assert_eq!(first_failure(&validator, &content), None);
    assert!(
        first_failure(&validator, &json!({ "when": 7 })).is_some(),
        "control: the field's type is still enforced"
    );
}

/// `format` is accepted in every schema position: the root, a `properties` value, an `items`
/// schema and an `additionalProperties` schema, and nested at depth.
#[test]
fn format_is_accepted_in_every_schema_position() {
    for schema in [
        field_tree(json!({ "type": "string", "format": "date" })),
        field_tree(json!({ "properties": { "a": { "format": "date" } } })),
        field_tree(json!({ "type": "array", "items": { "format": "date" } })),
        field_tree(json!({ "additionalProperties": { "format": "date" } })),
        field_tree(
            json!({ "properties": { "a": { "items": { "properties": { "b": { "format": "x" } } } } } }),
        ),
    ] {
        assert_compiles(&schema);
    }
}

/// A `format` must be a string; any other value is refused by the walk, naming it.
#[test]
fn a_format_that_is_not_a_string_is_refused() {
    assert_eq!(
        refusal(&field_tree(
            json!({ "properties": { "a": { "format": 7 } } })
        )),
        "keyword format that is not a string at /properties/a"
    );
}

/// A key beginning `x-` is accepted in every schema position and its value is never walked: an
/// object holding refused keywords, a scalar, an array, and a key that is only the prefix.
#[test]
fn an_extension_key_is_carried_in_every_position_and_never_walked() {
    let held = json!({ "$ref": "#", "allOf": [{}], "properties": 7, "$schema": "draft-07" });
    for value in [held, json!(7), json!([1, "two"]), json!(null)] {
        for schema in [
            field_tree(json!({ "x-a": value.clone() })),
            field_tree(json!({ "properties": { "a": { "x-": value.clone() } } })),
            field_tree(json!({ "items": { "x-order": value.clone() } })),
            field_tree(json!({ "additionalProperties": { "x-ui": value.clone() } })),
        ] {
            assert_compiles(&schema);
        }
    }
}

/// Only the `x-` prefix is inert: `X-` and `xa` are unknown keywords like any other.
#[test]
fn only_the_lowercase_x_dash_prefix_is_inert() {
    assert_eq!(
        refusal(&field_tree(json!({ "X-ui": 1 }))),
        "keyword X-ui at the root"
    );
    assert_eq!(
        refusal(&field_tree(json!({ "xui": 1 }))),
        "keyword xui at the root"
    );
}

// ── A refusal names the first keyword and path that failed (shape ruling R4) ───────────────

/// The refusal names the keyword and the JSON-pointer path of the schema position holding it, and
/// the detailed sentence keeps the fixed explanation after it.
#[test]
fn the_refusal_names_the_keyword_and_its_path() {
    let schema = field_tree(json!({
        "type": "object",
        "properties": { "when": { "allOf": [{ "type": "string" }] } }
    }));
    assert_eq!(refusal(&schema), "keyword allOf at /properties/when");
    let detailed = compile(&schema).expect_err("refused").detailed();
    let fixed = ShapeSchemaUncompilable::default().to_string();
    assert!(
        detailed.starts_with(
            "the shape's JSON Schema cannot be compiled (keyword allOf at /properties/when): a \
             shape is a field tree"
        ),
        "{detailed}"
    );
    let explanation = fixed
        .split_once(": ")
        .expect("the fixed sentence explains after a colon")
        .1;
    assert!(detailed.ends_with(explanation), "{detailed}");
}

/// Paths step through `properties`, `items` and `additionalProperties`, and a name holding `~` or
/// `/` is escaped as RFC 6901 says.
#[test]
fn the_path_names_every_kind_of_step_and_escapes_names() {
    let schema = field_tree(json!({
        "properties": { "a/b~c": { "items": { "additionalProperties": { "not": {} } } } }
    }));
    assert_eq!(
        refusal(&schema),
        "keyword not at /properties/a~1b~0c/items/additionalProperties"
    );
}

/// **First** is the walk's order: a position's own keywords before the positions nested in it,
/// and nested positions in the map's order. Of three refusals the first is named, and only it.
#[test]
fn the_first_refusal_in_the_walks_order_is_named() {
    let schema = field_tree(json!({
        "$defs": {},
        "properties": {
            "a": { "oneOf": [] },
            "b": { "anyOf": [] }
        }
    }));
    assert_eq!(refusal(&schema), "keyword $defs at the root");
    let nested = field_tree(json!({
        "properties": { "a": { "oneOf": [] }, "b": { "anyOf": [] } }
    }));
    assert_eq!(refusal(&nested), "keyword oneOf at /properties/a");
    let deeper = field_tree(json!({
        "properties": { "a": { "items": { "if": {} } }, "b": { "anyOf": [] } }
    }));
    assert_eq!(
        refusal(&deeper),
        "keyword if at /properties/a/items",
        "depth first: a's nested position before b"
    );
}

/// Each cap failure names the cap and the path.
#[test]
fn a_cap_failure_names_the_cap_and_the_path() {
    let strings: Vec<String> = (0..=MAX_ENUM_ENTRIES).map(|i| format!("v{i}")).collect();
    assert_eq!(
        refusal(&field_tree(
            json!({ "properties": { "a": { "enum": strings } } })
        )),
        format!(
            "enum of {} entries, past the cap of {MAX_ENUM_ENTRIES}, at /properties/a",
            MAX_ENUM_ENTRIES + 1
        )
    );
    let mixed: Vec<Value> = (0..=MAX_MIXED_ENUM_ENTRIES).map(|i| json!(i)).collect();
    assert_eq!(
        refusal(&field_tree(json!({ "items": { "enum": mixed } }))),
        format!(
            "enum of {} entries, past the cap of {MAX_MIXED_ENUM_ENTRIES}, at /items",
            MAX_MIXED_ENUM_ENTRIES + 1
        )
    );
    let source = "a".repeat(MAX_PATTERN_SOURCE_BYTES + 1);
    assert_eq!(
        refusal(&field_tree(
            json!({ "properties": { "a": { "pattern": source } } })
        )),
        format!(
            "pattern of {} bytes, past the cap of {MAX_PATTERN_SOURCE_BYTES} bytes of source, at \
             /properties/a",
            MAX_PATTERN_SOURCE_BYTES + 1
        )
    );
    let fields: serde_json::Map<String, Value> = (0..=MAX_SHAPE_PATTERNS)
        .map(|i| (format!("p{i:02}"), json!({ "pattern": "^a" })))
        .collect();
    assert_eq!(
        refusal(&field_tree(json!({ "properties": fields }))),
        format!(
            "pattern past the cap of {MAX_SHAPE_PATTERNS} in a shape at /properties/p{MAX_SHAPE_PATTERNS}"
        )
    );
    let name = "k".repeat(MAX_PROPERTY_NAME_BYTES + 1);
    assert_eq!(
        refusal(&field_tree(
            json!({ "properties": { "a": { "properties": { name.clone(): {} } } } })
        )),
        format!("a name in properties past {MAX_PROPERTY_NAME_BYTES} bytes at /properties/a")
    );
    assert_eq!(
        refusal(&field_tree(json!({ "required": [name] }))),
        format!("a name in required past {MAX_PROPERTY_NAME_BYTES} bytes at the root")
    );
    // Seventeen steps of `/properties/a` are past the room the bound leaves, so the path loses its
    // middle and keeps both ends.
    let too_deep = refusal(&chain(&[Step::Property; MAX_SHAPE_DEPTH + 1]));
    assert!(
        too_deep.starts_with(&format!(
            "nesting past {MAX_SHAPE_DEPTH} levels at /properties/a/"
        )) && too_deep.ends_with("/properties/a/properties/a")
            && too_deep.contains('…'),
        "{too_deep}"
    );
}

/// The refusal never quotes a value under `const`, `enum`, `default`, `examples` or an `x-` key,
/// and never a `$schema` it refused: only the schema's keywords, counts, caps and path.
#[test]
fn the_refusal_quotes_no_value() {
    const SENTINEL: &str = "SENTINEL-content-9d1e";
    let schema = field_tree(json!({
        "properties": {
            "a": {
                "const": SENTINEL,
                "enum": [SENTINEL],
                "default": SENTINEL,
                "examples": [SENTINEL],
                "x-note": SENTINEL,
                "allOf": []
            }
        }
    }));
    let error = compile(&schema).expect_err("refused");
    assert_eq!(error.refused(), Some("keyword allOf at /properties/a"));
    assert!(!error.detailed().contains(SENTINEL), "{}", error.detailed());
    let wrong_draft = json!({ "$schema": format!("https://{SENTINEL}/schema") });
    let error = compile(&wrong_draft).expect_err("refused");
    assert!(!error.detailed().contains(SENTINEL), "{}", error.detailed());
}

/// However long the path or the keyword, the detailed sentence fits the failure-message bound and
/// keeps the explanation whole: what was refused is cut, never the sentence after it.
#[test]
fn the_detailed_refusal_is_bounded_and_keeps_the_explanation() {
    let long = "n".repeat(MAX_PROPERTY_NAME_BYTES);
    let mut node = json!({ "allOf": [] });
    for _ in 0..MAX_SHAPE_DEPTH {
        node = json!({ "properties": { long.clone(): node } });
    }
    let fixed = ShapeSchemaUncompilable::default().to_string();
    let explanation = fixed.split_once(": ").expect("explained").1.to_owned();
    for schema in [field_tree(node), field_tree(json!({ "k".repeat(4096): 1 }))] {
        let error = compile(&schema).expect_err("refused");
        let detailed = error.detailed();
        assert!(
            detailed.len() <= MAX_FAILURE_MESSAGE_BYTES,
            "{}",
            detailed.len()
        );
        assert!(detailed.ends_with(&explanation), "{detailed}");
        assert!(
            error.refused().is_some_and(|r| r.contains('…')),
            "{detailed}"
        );
    }
}

/// A property name or an unknown key carrying control or bidirectional-formatting characters is
/// named with them escaped, so a refusal shown to another reader cannot drive a terminal or
/// reorder the sentence.
#[test]
fn control_and_bidi_characters_in_a_named_refusal_are_escaped() {
    let schema = field_tree(json!({
        "properties": { "a\u{1b}[31m\n\u{202e}b": { "allOf": [] } }
    }));
    let refused = refusal(&schema);
    assert_eq!(
        refused,
        "keyword allOf at /properties/a\\u{1b}[31m\\u{a}\\u{202e}b"
    );
    assert!(!refused.chars().any(char::is_control), "{refused:?}");
    assert_eq!(
        refusal(&field_tree(json!({ "\u{7}bell": 1 }))),
        "keyword \\u{7}bell at the root"
    );
}

/// The fixed sentence leaves room under the bound to name what was refused. Bites a sentence
/// grown so long that every refusal would silently name nothing.
#[test]
fn the_detailed_refusal_has_room() {
    use temper_substrate::shape_schema::{refused_room, MIN_REFUSED_ROOM};
    assert!(
        refused_room() >= MIN_REFUSED_ROOM,
        "{} bytes",
        refused_room()
    );
}

/// A refusal the build gives — 2020-12 meta-validation, or a pattern the regex engine will not
/// build — names a position whose property names are escaped as the walk's are. Bites a
/// `build_refusal` that quotes the build's pointer raw.
#[test]
fn control_and_bidi_characters_in_a_build_refusal_are_escaped() {
    let name = "a\u{1b}[31m\n\u{202e}b";
    let meta = refusal(&field_tree(
        json!({ "properties": { name: { "type": 5 } } }),
    ));
    assert_eq!(
        meta,
        "a value that is not valid 2020-12 at /properties/a\\u{1b}[31m\\u{a}\\u{202e}b/type"
    );
    let regex = refusal(&field_tree(
        json!({ "properties": { name: { "pattern": "(" } } }),
    ));
    assert!(
        regex.ends_with("at /properties/a\\u{1b}[31m\\u{a}\\u{202e}b"),
        "{regex}"
    );
    for refused in [&meta, &regex] {
        assert!(!refused.chars().any(char::is_control), "{refused:?}");
        assert!(!refused.contains('\u{202e}'), "{refused:?}");
    }
}
