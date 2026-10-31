//! A benchmark of the worst-case cost of a shape's check and compile under the caps
//! (`MAX_MIXED_ENUM_ENTRIES`, `MAX_SHAPE_PATTERNS`, `MAX_PATTERN_SOURCE_BYTES`,
//! `MAX_PATTERN_BYTES`, `MAX_SHAPE_DEPTH`, the node budget's `MAX_SHAPE_NODES` and
//! `MAX_SHAPE_OBJECTS`, and the walk's `MAX_SCHEMA_BYTES`). Each test builds a schema that sits AT
//! the caps, then times one operation with `std::time::Instant` and prints `name: <ms> ms`:
//!
//! - `check_mixed_enum_at_the_cap`: `first_failure` of a 4.5 MB array against an `enum` of
//!   `MAX_MIXED_ENUM_ENTRIES` scalars, integers and strings alternating.
//! - `check_dfa_defeating_pattern` (and its anchored twin): `first_failure` of one 4.5 MB string
//!   of `a`/`b` against `(a|b)*a(a|b){20}`, a pattern whose DFA has a state for every window of
//!   the last 21 characters.
//! - `compile_at_the_pattern_cap`: `compile` of a schema of `MAX_SCHEMA_BYTES` holding
//!   `MAX_SHAPE_PATTERNS` distinct patterns, each compiling to just under `MAX_PATTERN_BYTES`.
//! - `check_dfa_pattern_at_the_cap`: the same a/b string against `^(a|b)*a(a|b){DFA_REPEAT}$`.
//! - `check_long_property_name_at_the_cap`: 4.5 MB of two-key objects under `items`, against a
//!   `properties` key of `MAX_PROPERTY_NAME_BYTES`.
//! - `many_tiny_subschemas_at_the_size_cap`: schemas built of as many small subschemas as fit both
//!   `MAX_SCHEMA_BYTES` and the node budget — empty fields, typed fields with `required` and
//!   `additionalProperties: false`, each filling the budget, and trees of 2, 4 and 16 children a
//!   node, each the deepest complete tree within it — each timed for `compile`, a check of
//!   content holding every key, and the ask walk's `enumerate`, which no visit budget bounds. Two
//!   more are walked whole without reaching `MAX_WALK_PATHS`, each filling the budget: fields that
//!   are each `false`, which the walk reads and never counts, and fields each carrying an enum of
//!   `MAX_ENUM_ENTRIES` strings.
//! - `parse_worst_case_at_the_byte_cap`: a schema of `MAX_SCHEMA_BYTES` whose `examples` hold as
//!   many `{"":0}` as fit, at the root and at the foot of an `items` chain, far past the node
//!   budget; times parsing its text, cloning the parsed `Value` and `compile`'s refusal.
//! - `parse_worst_case_at_the_caps`: a schema of `MAX_SCHEMA_BYTES` holding exactly
//!   `MAX_SHAPE_NODES` values, exactly `MAX_SHAPE_OBJECTS` of them objects, all but the root
//!   `{"":0}`; times parsing its text, cloning the parsed `Value` and `compile`, and prints the
//!   schema's `parsed_weight`.
//! - `compile_admission_profiles`: `compile` of each synthetic admission profile in
//!   `MAX_SHAPE_NODES`'s doc.
//! - `vocabulary_values_worst_case`: the vocabulary's values term at the node budget, two ways —
//!   fields each an `enum` of `MAX_ENUM_ENTRIES` strings, and leaves under a spine of keys of
//!   `MAX_PROPERTY_NAME_BYTES` each declaring an `enum` of `MAX_MIXED_ENUM_ENTRIES` numbers at
//!   three `items` levels; times `enumerate`.
//! - `vocabulary_values_structured_worst_case`: a schema no shape can now hold, walk-only: as many
//!   fields as fit, each an `enum` of `MAX_MIXED_ENUM_ENTRIES` one-key objects; times `enumerate`,
//!   which copies every member.
//! - `vocabulary_paths_worst_case`: `MAX_WALK_PATHS` paths under a spine of keys of
//!   `MAX_PROPERTY_NAME_BYTES`; times `enumerate`, which copies the spine into every path.
//! - `refuse_past_the_node_budget`: `within_node_budget` of a schema one value past the node cap,
//!   of one of `MAX_SCHEMA_BYTES` whose excess lies past where the count stops, and of one of
//!   `MAX_SCHEMA_BYTES` whose excess is one array.
//!
//! Run before merging a change to a cap (release, so the numbers are the deployed ones):
//!
//! ```text
//! cargo test -p temper-substrate --all-features --release \
//!     --test shape_schema_cost_bench -- --ignored --nocapture
//! ```
//!
//! Always `-p temper-substrate`, never `--workspace`: a workspace build unifies serde_json's
//! `preserve_order` in, which measures a different, lighter `Value` than the deployed one. Every
//! test asserts the deployed size first ([`deployed_value`]).
//!
//! Peak memory is not measured here (that would need a counting allocator, which needs `unsafe`):
//! run each test alone under `/usr/bin/time -l` and read "maximum resident set size". The two parse
//! cases and the three vocabulary cases stop early under `SHAPE_BENCH_STOP` ([`stops_at`]), so the
//! same test run as a control isolates what its measured step holds. A case with more than one
//! fixture runs one alone under `SHAPE_BENCH_CASE`, its fixture's name.
//!
//! Every test is `#[ignore]` and asserts nothing about time: only that the construction is what it
//! claims (the schema compiles or is refused, the check runs to a verdict). Needs no database.

use std::time::Instant;

use serde_json::{json, Value};
use temper_core::types::artifact_ask::walk::{MAX_SCHEMA_BYTES, MAX_WALK_DEPTH, MAX_WALK_PATHS};
use temper_core::types::artifact_ask::{enumerate, WalkBounds};
use temper_substrate::shape_schema::{
    compile, count_up_to, first_failure, parsed_weight, within_node_budget,
    ShapeSchemaUncompilable, MAX_ENUM_ENTRIES, MAX_MIXED_ENUM_ENTRIES, MAX_PATTERN_SOURCE_BYTES,
    MAX_PROPERTY_NAME_BYTES, MAX_SHAPE_DEPTH, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS,
    MAX_SHAPE_PARSED_WEIGHT, MAX_SHAPE_PATTERNS,
};

/// The compact-JSON size of the content the check benchmarks validate: 4.5 MB, near what the
/// platform's request body admits for one artifact's content.
const PLATFORM_BODY_BYTES: usize = 4_500_000;

/// The root `$schema` every benchmark schema names.
const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// The repeat bound `k` in `^[\w.-]{1,k}x<NN>$` for `compile_at_the_pattern_cap`. The controller
/// tunes it by measurement so each pattern compiles near but under `MAX_PATTERN_BYTES` (10 KiB):
/// `k = 55` is about 8.7 KB on regex 1.13.1.
const PATTERN_REPEAT: usize = 55;

/// The `N` in `^(a|b)*a(a|b){N}$` for `check_dfa_pattern_at_the_cap`: the largest repeat whose
/// pattern still compiles under `MAX_PATTERN_BYTES`. The controller tunes it by measurement.
const DFA_REPEAT: usize = 70;

/// Levels of `properties` between the array's items and the schema that holds the `enum`.
const NESTING: usize = 4;

/// Milliseconds since `start`, printed as `name: <ms> ms`.
fn report(name: &str, start: Instant) {
    println!("{name}: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
}

/// Asserts the `Value` under measurement is the deployed one: 32 bytes, serde_json without
/// `preserve_order`. Every test calls it first.
fn deployed_value() {
    assert_eq!(
        std::mem::size_of::<Value>(),
        32,
        "serde_json::Value is not the deployed one: a `--workspace` build unifies in \
         `preserve_order`, which measures a different, lighter Value. Run with `-p temper-substrate`"
    );
}

/// How far a case runs, read from `SHAPE_BENCH_STOP`, so that `/usr/bin/time -l` of the same test
/// at each stage isolates what one step holds:
///
/// - `input`: stop once the case's input exists (the text, or the schema built in code).
/// - `parse`: stop once that text is also parsed — the control a parse case is read against. A
///   case with no parse stops at its input.
/// - `measured`: stop once the measured step has run, before any check that allocates on its own.
///
/// Unset or empty, the whole case runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Input,
    Parse,
    Measured,
}

/// Whether the case should stop after `stage`.
fn stops_at(stage: Stage) -> bool {
    let stop = match std::env::var("SHAPE_BENCH_STOP").as_deref() {
        Ok("input") => Stage::Input,
        Ok("parse") => Stage::Parse,
        Ok("measured") => Stage::Measured,
        Ok("") | Err(_) => return false,
        Ok(other) => panic!("SHAPE_BENCH_STOP={other}: expected input, parse or measured"),
    };
    stop <= stage
}

/// `inner` wrapped in `levels` single-property object schemas: `{"properties":{"l1":{...}}}`.
fn nest_schema(inner: Value, levels: usize) -> Value {
    (1..=levels).rev().fold(
        inner,
        |schema, level| json!({ "type": "object", "properties": { format!("l{level}"): schema } }),
    )
}

/// `inner` wrapped in `levels` single-key objects, the content that fits `nest_schema`.
fn nest_content(inner: Value, levels: usize) -> Value {
    (1..=levels).rev().fold(
        inner,
        |content, level| json!({ format!("l{level}"): content }),
    )
}

/// The enum entry at `index`: an integer at an even index, a string at an odd one, so the enum is
/// mixed and every entry distinct. `entry(MAX_MIXED_ENUM_ENTRIES)` is in no enum of the cap's
/// entries.
fn entry(index: usize) -> Value {
    if index.is_multiple_of(2) {
        json!(index)
    } else {
        json!(format!("v{index}"))
    }
}

fn assert_compact_size_within(value: &Value, floor: usize, ceiling: usize, what: &str) {
    let len = serde_json::to_vec(value).expect("serializes").len();
    assert!(
        (floor..=ceiling).contains(&len),
        "{what}: {len} compact bytes, expected {floor}..={ceiling}"
    );
}

/// The root's `items` is an object schema nested `NESTING` levels deep whose innermost schema
/// holds an `enum` of `MAX_MIXED_ENUM_ENTRIES` scalars (a mixed enum, scanned entry by entry). The
/// content is 4.5 MB of elements whose innermost value is the last entry (each is a valid element
/// and costs a scan of the whole enum) except for the final element, whose innermost value is in
/// no entry: the check runs through every element and fails there.
///
/// On `main` at `0ef10cb0d`, where each entry was a one-key object holding 16 integers (an enum no
/// shape can now hold), arm64 (M4 Pro), rustc 1.99, release, median of three runs: 87 ms (80–91);
/// max RSS 240 MiB.
///
/// With the node budget, arm64 (M4 Pro), rustc 1.99, release, median of three runs, with 32 mixed
/// scalars: 38 ms; max RSS 361 MiB.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn check_mixed_enum_at_the_cap() {
    deployed_value();
    const _: () = assert!(
        NESTING < MAX_SHAPE_DEPTH,
        "the schema is within the depth cap"
    );
    let entries: Vec<Value> = (0..MAX_MIXED_ENUM_ENTRIES).map(entry).collect();
    assert_eq!(entries.len(), MAX_MIXED_ENUM_ENTRIES);
    let schema = json!({
        "$schema": DRAFT,
        "type": "array",
        "items": nest_schema(json!({ "enum": entries }), NESTING),
    });
    let validator = compile(&schema).expect("the enum schema compiles at the caps");

    let element = nest_content(entry(MAX_MIXED_ENUM_ENTRIES - 1), NESTING);
    let element_len = serde_json::to_vec(&element).expect("serializes").len();
    // `[e,e,...,e]` is n * (element_len + 1) + 1 bytes.
    let count = (PLATFORM_BODY_BYTES - 1) / (element_len + 1);
    let mut items = vec![element; count];
    *items.last_mut().expect("at least one element") =
        nest_content(entry(MAX_MIXED_ENUM_ENTRIES), NESTING);
    let content = Value::Array(items);
    assert_compact_size_within(
        &content,
        PLATFORM_BODY_BYTES - (element_len + 1),
        PLATFORM_BODY_BYTES,
        "enum content",
    );

    let start = Instant::now();
    let verdict = first_failure(&validator, &content);
    report("check_mixed_enum_at_the_cap", start);
    assert!(verdict.is_some(), "the last element is in no enum entry");
}

/// A string of `a`/`b` of `PLATFORM_BODY_BYTES` compact bytes, drawn from a fixed-seed LCG
/// (Knuth's MMIX constants; the high bit of the state picks each character).
fn ab_string() -> Value {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let text: String = (0..PLATFORM_BODY_BYTES - 2)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            if state >> 63 == 0 {
                'a'
            } else {
                'b'
            }
        })
        .collect();
    let content = Value::String(text);
    assert_compact_size_within(
        &content,
        PLATFORM_BODY_BYTES,
        PLATFORM_BODY_BYTES,
        "ab string",
    );
    content
}

fn time_pattern_check(name: &str, pattern: &str) {
    assert!(
        pattern.len() <= MAX_PATTERN_SOURCE_BYTES,
        "within the source cap"
    );
    let schema = json!({ "$schema": DRAFT, "type": "string", "pattern": pattern });
    let validator = compile(&schema).expect("the pattern compiles");
    let content = ab_string();

    let start = Instant::now();
    let verdict = first_failure(&validator, &content);
    report(name, start);
    // Either verdict is a finished check; the point is the time it took to get there.
    drop(verdict);
}

/// `(a|b)*a(a|b){20}` as written: its DFA needs a state for each of the 2^21 windows of the last
/// 21 characters. Unanchored, a search may find a match early and stop; the anchored twin below
/// cannot.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn check_dfa_defeating_pattern() {
    deployed_value();
    time_pattern_check("check_dfa_defeating_pattern", "(a|b)*a(a|b){20}");
}

/// The same pattern anchored at both ends, so a match must be decided at the end of the string
/// and the engine must read all 4.5 MB.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn check_dfa_defeating_pattern_anchored() {
    deployed_value();
    time_pattern_check("check_dfa_defeating_pattern_anchored", "^(a|b)*a(a|b){20}$");
}

/// `MAX_SHAPE_PATTERNS` properties, each with its own pattern `^[\w.-]{1,k}x<NN>$` (a distinct literal
/// suffix), padded with `description` text so the compact schema is `MAX_SCHEMA_BYTES`. Times
/// `compile` alone.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn compile_at_the_pattern_cap() {
    deployed_value();
    let build = |padding: usize| {
        let properties: serde_json::Map<String, Value> = (0..MAX_SHAPE_PATTERNS)
            .map(|index| {
                let pattern = format!(r"^[\w.-]{{1,{PATTERN_REPEAT}}}x{index:02}$");
                (
                    format!("field_{index:02}"),
                    json!({ "type": "string", "pattern": pattern }),
                )
            })
            .collect();
        json!({
            "$schema": DRAFT,
            "type": "object",
            "description": "x".repeat(padding),
            "properties": properties,
        })
    };
    let bare = serde_json::to_vec(&build(0)).expect("serializes").len();
    assert!(
        bare <= MAX_SCHEMA_BYTES,
        "the patterns alone fit the schema bound"
    );
    let schema = build(MAX_SCHEMA_BYTES - bare);
    assert_compact_size_within(&schema, MAX_SCHEMA_BYTES, MAX_SCHEMA_BYTES, "padded schema");

    let start = Instant::now();
    let compiled = compile(&schema);
    report("compile_at_the_pattern_cap", start);
    assert!(
        compiled.is_ok(),
        "every pattern compiles under MAX_PATTERN_BYTES; tune PATTERN_REPEAT"
    );
}

/// The same a/b string against the largest DFA-defeating pattern that compiles under
/// `MAX_PATTERN_BYTES`.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn check_dfa_pattern_at_the_cap() {
    deployed_value();
    let pattern = format!("^(a|b)*a(a|b){{{DFA_REPEAT}}}$");
    time_pattern_check("check_dfa_pattern_at_the_cap", &pattern);
}

/// A `properties` key of `MAX_PROPERTY_NAME_BYTES` under `items`; the content is 4.5 MB of
/// `{"a":0,"b":0}` objects, so each object costs a lookup of the long key.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn check_long_property_name_at_the_cap() {
    deployed_value();
    let schema = json!({
        "$schema": DRAFT,
        "type": "array",
        "items": { "type": "object", "properties": {
            "k".repeat(MAX_PROPERTY_NAME_BYTES): { "type": "integer" }
        } },
    });
    let validator = compile(&schema).expect("a name at the cap compiles");
    let element = json!({ "a": 0, "b": 0 });
    let element_len = serde_json::to_vec(&element).expect("serializes").len();
    let count = (PLATFORM_BODY_BYTES - 1) / (element_len + 1);
    let content = Value::Array(vec![element; count]);
    let start = Instant::now();
    let verdict = first_failure(&validator, &content);
    report("check_long_property_name_at_the_cap", start);
    assert!(verdict.is_none(), "no object holds the long key");
}

/// The values and the objects `value` holds, counted whole.
fn counted(value: &Value) -> (usize, usize) {
    let count = count_up_to(value, usize::MAX, usize::MAX);
    (count.nodes, count.objects)
}

/// The most fields, each holding `per_field`'s values and objects, that fit the node budget beside
/// a frame holding `frame`'s.
fn fields_in_the_budget(frame: (usize, usize), per_field: (usize, usize)) -> usize {
    let by_values = (MAX_SHAPE_NODES - frame.0) / per_field.0;
    let by_objects = (MAX_SHAPE_OBJECTS - frame.1)
        .checked_div(per_field.1)
        .unwrap_or(usize::MAX);
    by_values.min(by_objects)
}

/// Asserts `schema` fills the node budget: it holds at most `MAX_SHAPE_NODES` values and at most
/// `MAX_SHAPE_OBJECTS` objects, and `unit` more — the values and objects of one more of the parts
/// it repeats — would pass one of them. So it sits within one part of a cap, and a fixture that
/// drifts short of the budget fails here. Prints the counts.
fn assert_fills_the_budget(schema: &Value, unit: (usize, usize), what: &str) {
    let (nodes, objects) = counted(schema);
    println!("{what}: {nodes} values, {objects} objects");
    assert!(
        nodes <= MAX_SHAPE_NODES && objects <= MAX_SHAPE_OBJECTS,
        "{what}: {nodes} values and {objects} objects are past the budget"
    );
    assert!(
        nodes + unit.0 > MAX_SHAPE_NODES || objects + unit.1 > MAX_SHAPE_OBJECTS,
        "{what}: {nodes} values and {objects} objects leave room for another part of {unit:?}"
    );
}

/// An object schema of as many fields `f00000`, `f00001`, … as fit both `MAX_SCHEMA_BYTES` and the
/// node budget, each `field`; with `closed`, every field is also `required` and
/// `additionalProperties` is `false`. Returns the schema and the values and objects one field adds.
fn wide_object(field: &Value, closed: bool) -> (Value, (usize, usize)) {
    let entry = serde_json::to_vec(&json!({ "f00000": field }))
        .expect("serializes")
        .len()
        - 1;
    let per = entry + if closed { r#""f00000","#.len() } else { 0 };
    let by_bytes = (MAX_SCHEMA_BYTES - 128) / per;
    // The root, `$schema`, `type` and `properties`; closed, `required` and `additionalProperties`
    // too. Closed, each field is also a `required` entry.
    let frame = if closed { (6, 2) } else { (4, 2) };
    let (values, objects) = counted(field);
    let per_field = (values + usize::from(closed), objects);
    let count = by_bytes.min(fields_in_the_budget(frame, per_field));
    let names: Vec<String> = (0..count).map(|i| format!("f{i:05}")).collect();
    let properties: serde_json::Map<String, Value> =
        names.iter().map(|n| (n.clone(), field.clone())).collect();
    let mut schema = json!({ "$schema": DRAFT, "type": "object", "properties": properties });
    if closed {
        schema["required"] = json!(names);
        schema["additionalProperties"] = json!(false);
    }
    (schema, per_field)
}

/// A complete tree of `children` children a node, `depth` levels below the root, each leaf
/// `{ "type": "integer" }`.
fn tree(children: usize, depth: usize) -> Value {
    if depth == 0 {
        return json!({ "type": "integer" });
    }
    let properties: serde_json::Map<String, Value> = (0..children)
        .map(|i| (format!("c{i}"), tree(children, depth - 1)))
        .collect();
    json!({ "type": "object", "properties": properties })
}

/// Content holding every key `schema` declares, at every level, with `0` at each leaf.
fn every_key(schema: &Value) -> Value {
    match schema.get("properties").and_then(Value::as_object) {
        Some(properties) => Value::Object(
            properties
                .iter()
                .map(|(key, child)| (key.clone(), every_key(child)))
                .collect(),
        ),
        None => json!(0),
    }
}

/// Time `compile`, a check of content holding every key, and `enumerate` of one schema.
fn time_at_the_size_cap(name: &str, schema: &Value) {
    assert_compact_size_within(schema, 1, MAX_SCHEMA_BYTES, name);
    let start = Instant::now();
    let validator = compile(schema).expect("a field tree at the size cap compiles");
    report(&format!("{name} compile"), start);
    let content = every_key(schema);
    let start = Instant::now();
    let verdict = first_failure(&validator, &content);
    report(&format!("{name} check"), start);
    assert!(
        verdict.is_none(),
        "{name}: content holding every key conforms"
    );
    let start = Instant::now();
    let vocabulary = enumerate(schema, &WalkBounds::BUILD);
    report(&format!("{name} enumerate"), start);
    assert!(!vocabulary.paths.is_empty(), "{name}: the walk reads it");
}

/// Schemas built of as many of the smallest subschemas a field tree allows as fit both
/// `MAX_SCHEMA_BYTES` and the node budget. The budget binds every one before the bytes do:
///
/// - each wide object holds as many fields as the budget takes, so one field more would pass a
///   cap ([`assert_fills_the_budget`]). Empty fields, typed fields and closed typed fields stop at
///   `MAX_SHAPE_OBJECTS`; true fields, false fields and enum-carrying fields at `MAX_SHAPE_NODES`.
/// - each tree is the deepest complete tree within the budget, and one level deeper passes it. A
///   complete tree cannot come closer: it grows by a whole level at a time. At the provisional caps
///   that is depth 10 for 2 children, 5 for 4 and 2 for 16.
///
/// The figures below are for the fixtures as they were before the node budget, each filling
/// `MAX_SCHEMA_BYTES`. The 2026-10-29 retro measured them, before this file held them, at compile
/// 7–30 ms and check 0.2–2.3 ms on a local release build. On `main` at `0ef10cb0d`, arm64 (M4 Pro),
/// rustc 1.99, release, median of three runs (max RSS of the whole test 156–167 MiB):
///
/// | schema | compile | check | enumerate |
/// |---|---|---|---|
/// | empty fields | 6.9 ms | 0.28 ms | 0.64 ms |
/// | true fields | 4.8 ms | 0.17 ms | 0.51 ms |
/// | typed fields | 4.0 ms | 0.12 ms | 0.54 ms |
/// | typed fields, closed | 3.4 ms | 0.54 ms | 0.50 ms |
/// | tree of 2 at depth 12 | 5.5 ms | 0.15 ms | 0.72 ms |
/// | tree of 4 at depth 6 | 2.9 ms | 0.08 ms | 0.39 ms |
/// | tree of 16 at depth 3 | 2.1 ms | 0.08 ms | 0.32 ms |
/// | false fields | 5.8 ms | — | 0.49 ms |
/// | enum-carrying fields | 1.3 ms | — | 0.60 ms |
///
/// With the node budget, arm64 (M4 Pro), rustc 1.99, release, median of three runs, each fixture
/// filled to the budget (max RSS of the whole test 31 MiB):
///
/// | schema | values / objects | compile | check | enumerate |
/// |---|---|---|---|---|
/// | empty fields | 3,074 / 3,072 | 1.35 ms | 0.03 ms | 0.16 ms |
/// | true fields | 12,288 / 2 | 3.09 ms | 0.11 ms | 0.34 ms |
/// | typed fields | 6,144 / 3,072 | 1.20 ms | 0.03 ms | 0.24 ms |
/// | typed fields, closed | 9,216 / 3,072 | 1.28 ms | 0.22 ms | 0.27 ms |
/// | tree of 2 at depth 10 | 5,118 / 3,070 | 1.22 ms | 0.03 ms | 0.29 ms |
/// | tree of 4 at depth 5 | 3,072 / 1,706 | 0.66 ms | 0.02 ms | 0.20 ms |
/// | tree of 16 at depth 2 | 564 / 290 | 0.13 ms | 0.01 ms | 0.06 ms |
/// | false fields | 12,288 / 2 | 3.30 ms | — | 0.33 ms |
/// | enum-carrying fields | 12,130 / 49 | 0.59 ms | — | 0.26 ms |
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn many_tiny_subschemas_at_the_size_cap() {
    deployed_value();
    for (name, field, closed) in [
        ("empty fields", json!({}), false),
        ("true fields", json!(true), false),
        ("typed fields", json!({ "type": "integer" }), false),
        ("typed fields, closed", json!({ "type": "integer" }), true),
    ] {
        let (schema, unit) = wide_object(&field, closed);
        assert_fills_the_budget(&schema, unit, name);
        time_at_the_size_cap(name, &schema);
    }
    for children in [2, 4, 16] {
        let rooted = |depth| {
            let mut schema = tree(children, depth);
            schema["$schema"] = json!(DRAFT);
            schema
        };
        let depth = (1..=MAX_SHAPE_DEPTH)
            .take_while(|&depth| {
                let schema = rooted(depth);
                let bytes = serde_json::to_vec(&schema).expect("serializes").len();
                bytes <= MAX_SCHEMA_BYTES - 128 && within_node_budget(&schema)
            })
            .last()
            .expect("one level fits");
        let schema = rooted(depth);
        let name = format!("tree of {children} at depth {depth}");
        let (nodes, objects) = counted(&schema);
        println!("{name}: {nodes} values, {objects} objects");
        assert!(
            within_node_budget(&schema) && !within_node_budget(&rooted(depth + 1)),
            "{name}: the deepest complete tree within the budget"
        );
        time_at_the_size_cap(&name, &schema);
    }
    let (schema, unit) = wide_object(&json!(false), false);
    time_a_whole_walk("false fields", &schema, 0, unit);
    let (schema, fields) = enum_fields();
    time_a_whole_walk("enum-carrying fields", &schema, fields, STRING_ENUM_FIELD);
}

/// The values and objects one field of [`string_enum_fields_text`] holds: the field, its `enum`
/// array and its `MAX_ENUM_ENTRIES` members, the field the one object.
const STRING_ENUM_FIELD: (usize, usize) = (MAX_ENUM_ENTRIES + 2, 1);

/// The text of a schema of as many fields as fit both the size cap and the node budget, each an
/// `enum` of `MAX_ENUM_ENTRIES` short distinct codes, and how many fields there are. Built as text,
/// never as a `Value`, so the input holds only the text.
fn string_enum_fields_text() -> (String, usize) {
    let members: Vec<String> = (0..MAX_ENUM_ENTRIES)
        .map(|c| format!(r#""c{c:05}""#))
        .collect();
    let field = format!(r#"{{"enum":[{}]}}"#, members.join(","));
    let head = format!(r#"{{"$schema":"{DRAFT}","type":"object","properties":{{"#);
    let per = format!(r#""f0000":{field},"#).len();
    let by_bytes = (MAX_SCHEMA_BYTES - head.len() - 2) / per;
    // The root, `$schema`, `type` and `properties`; the root and `properties` the objects.
    let fields = by_bytes.min(fields_in_the_budget((4, 2), STRING_ENUM_FIELD));
    let properties: Vec<String> = (0..fields)
        .map(|f| format!(r#""f{f:04}":{field}"#))
        .collect();
    (format!("{head}{}}}}}", properties.join(",")), fields)
}

/// [`string_enum_fields_text`]'s schema, parsed, and how many fields it holds.
fn enum_fields() -> (Value, usize) {
    let (text, fields) = string_enum_fields_text();
    let schema = serde_json::from_str(&text).expect("the schema text parses");
    (schema, fields)
}

/// Time `compile` and `enumerate` of a schema the walk reads to its end without a cut: the
/// vocabulary holds `paths` paths and was truncated at no bound. The schema fills the node budget,
/// within `unit` — one more of the parts it repeats — of a cap ([`assert_fills_the_budget`]).
fn time_a_whole_walk(name: &str, schema: &Value, paths: usize, unit: (usize, usize)) {
    assert_compact_size_within(schema, 1, MAX_SCHEMA_BYTES, name);
    assert_fills_the_budget(schema, unit, name);
    let start = Instant::now();
    compile(schema).expect("a field tree at the size cap compiles");
    report(&format!("{name} compile"), start);
    let start = Instant::now();
    let vocabulary = enumerate(schema, &WalkBounds::BUILD);
    report(&format!("{name} enumerate"), start);
    assert_eq!(vocabulary.paths.len(), paths, "{name}: every declared path");
    assert!(
        vocabulary.truncated.is_empty(),
        "{name}: walked whole, never cut"
    );
}

/// How many `items` levels the chained variant of `parse_worst_case_at_the_byte_cap` nests its
/// `examples` under: one short of `MAX_SHAPE_DEPTH`.
const ITEMS_CHAIN: usize = MAX_SHAPE_DEPTH - 1;

/// The text of a schema of exactly `MAX_SCHEMA_BYTES` compact bytes whose `examples` hold as many
/// `{"":0}` as fit, at the foot of `chain` levels of `{"type":"array","items":…}`, and how many
/// objects the `examples` hold. Built as text, never as a `Value`, so the input holds only the text.
fn tiny_objects_text(chain: usize) -> (String, usize) {
    const TINY: &str = r#"{"":0}"#;
    // The root names the draft and carries the padding; the `examples` sit `chain` levels below.
    let root = format!(r#"{{"$schema":"{DRAFT}","description":""#);
    let (open, close) = if chain == 0 {
        (
            r#"","type":"object","examples":["#.to_owned(),
            "]}".to_owned(),
        )
    } else {
        (
            format!(
                r#"","type":"array","items":{}{{"type":"object","examples":["#,
                r#"{"type":"array","items":"#.repeat(chain - 1)
            ),
            format!("]}}{}", "}".repeat(chain)),
        )
    };
    let frame = root.len() + open.len() + close.len();
    // `n` objects and `n - 1` commas between them; the `description` takes what is left.
    let count = (MAX_SCHEMA_BYTES - frame + 1) / (TINY.len() + 1);
    let objects = vec![TINY; count].join(",");
    let pad = "x".repeat(MAX_SCHEMA_BYTES - frame - objects.len());
    let text = [root.as_str(), &pad, &open, &objects, &close].concat();
    assert_eq!(
        text.len(),
        MAX_SCHEMA_BYTES,
        "the schema text is at the byte cap"
    );
    (text, count)
}

/// The stored schema's own `Value` is the largest thing a shape read allocates. Tiny objects are
/// its worst case per byte: each holds a whole map node however few members it has. A schema of
/// `MAX_SCHEMA_BYTES` of them is far past `MAX_SHAPE_OBJECTS`, so `compile` refuses it. Times
/// parsing the text (`T_parse`), cloning the parsed `Value` as the loader does before a compile
/// (`T_clone`), and the refusal (`T_refuse`), for the `examples` at the root and at the foot of an
/// `ITEMS_CHAIN`-deep `items` chain.
///
/// On `main` at `0ef10cb0d`, where only `MAX_SCHEMA_BYTES` bounded such a schema and `compile`
/// admitted it (`T_compile` is that compile), arm64 (M4 Pro), rustc 1.99, release, median of three
/// runs; both placements measure the same:
///
/// | | root (37,434 objects) | items chain (37,380) |
/// |---|---|---|
/// | `T_parse` | 2.1 ms | 2.1 ms |
/// | `T_clone` | 2.7 ms | 2.7 ms |
/// | `T_compile` | 4.0 ms | 4.0 ms |
/// | max RSS at `input` | 6.9 MiB | 6.9 MiB |
/// | max RSS at `parse` | 31.1 MiB | 31.1 MiB |
/// | max RSS at `measured` | 83.3 MiB | 83.5 MiB |
///
/// So the parsed `Value` holds 24.2 MiB, and the clone and the compile together 52 MiB more: the
/// clone is another 24.2, so the compile itself holds about 28 MiB.
///
/// With the node budget, arm64 (M4 Pro), rustc 1.99, release, median of three runs: the schema is
/// refused; `T_parse` 2.0 ms, `T_clone` 2.35 ms, `T_refuse` 0.02 ms; max RSS at `measured` 55.8 MiB
/// for either placement, against 83.3 MiB on `main`, because nothing compiles it.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn parse_worst_case_at_the_byte_cap() {
    deployed_value();
    let case = std::env::var("SHAPE_BENCH_CASE").unwrap_or_default();
    for (name, chain) in [("root", 0), ("items chain", ITEMS_CHAIN)] {
        if !case.is_empty() && case != name {
            continue;
        }
        let (text, objects) = tiny_objects_text(chain);
        println!("parse worst case, {name}: {objects} objects in examples");
        if stops_at(Stage::Input) {
            continue;
        }
        let start = Instant::now();
        let schema: Value = serde_json::from_str(&text).expect("the schema text parses");
        report(&format!("parse worst case, {name}: T_parse"), start);
        if stops_at(Stage::Parse) {
            continue;
        }
        let start = Instant::now();
        let copy = schema.clone();
        report(&format!("parse worst case, {name}: T_clone"), start);
        let start = Instant::now();
        let compiled = compile(&copy);
        report(&format!("parse worst case, {name}: T_refuse"), start);
        if stops_at(Stage::Measured) {
            continue;
        }
        assert_eq!(
            compiled.err(),
            Some(ShapeSchemaUncompilable),
            "{name}: {objects} objects are past the node budget"
        );
    }
}

/// The text of a schema at both caps and the byte cap: exactly `MAX_SHAPE_NODES` JSON values,
/// exactly `MAX_SHAPE_OBJECTS` of them objects, and `MAX_SCHEMA_BYTES` compact bytes. Its
/// `examples` hold `MAX_SHAPE_OBJECTS - 1` tiny objects `{"":0}`, each holding a map node, then the
/// zeros that bring the values to the node cap; a `description` takes the bytes left. Built as
/// text, never as a `Value`, so the input holds only the text.
fn at_the_caps_text() -> String {
    const TINY: &str = r#"{"":0}"#;
    // Beside the `examples`' members: the root, `$schema`, `description`, `type` and the `examples`
    // array, the root the one object among them.
    const FRAME_VALUES: usize = 5;
    let tiny = MAX_SHAPE_OBJECTS - 1;
    // Each tiny object is two values, itself and its member.
    let zeros = MAX_SHAPE_NODES - FRAME_VALUES - 2 * tiny;
    let mut members = vec![TINY; tiny];
    members.resize(tiny + zeros, "0");
    let head = format!(r#"{{"$schema":"{DRAFT}","description":""#);
    let tail = format!(r#"","type":"object","examples":[{}]}}"#, members.join(","));
    let pad = "x".repeat(MAX_SCHEMA_BYTES - head.len() - tail.len());
    let text = [head.as_str(), &pad, &tail].concat();
    assert_eq!(
        text.len(),
        MAX_SCHEMA_BYTES,
        "the schema text is at the byte cap"
    );
    text
}

/// The heaviest schema a shape can now hold, by the weight model: both caps full and the text at
/// `MAX_SCHEMA_BYTES`, so [`parsed_weight`] is the model's bound. Its objects are tiny objects, the
/// worst case per byte, since each holds a map node; the zeros beside them each take a slot of the
/// `examples` array; the bytes left go to one long `description`, which fits. Times parsing the
/// text (`T_parse`), cloning the parsed `Value` as the loader does before a compile (`T_clone`),
/// and `compile` (`T_compile`), and prints the model's weight beside `MAX_SHAPE_PARSED_WEIGHT`, the
/// target the measured RSS is read against.
///
/// With the node budget, arm64 (M4 Pro), rustc 1.99, release, median of three runs: `parsed_weight`
/// 4,550,656 bytes against the 4,718,592 target; `T_parse` 0.26 ms, `T_clone` 0.15 ms, `T_compile`
/// 0.85 ms; max RSS 6.8 MiB at `input`, 9.1 MiB at `parse` (the parsed `Value` holds 2.3 MiB, half
/// what the model weighs), 16.4 MiB at `measured` (the clone and the compile hold 7.3 MiB more).
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn parse_worst_case_at_the_caps() {
    deployed_value();
    let text = at_the_caps_text();
    let weight = parsed_weight(MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS, text.len());
    println!(
        "parse worst case at the caps: parsed_weight {weight} bytes, target \
         MAX_SHAPE_PARSED_WEIGHT {MAX_SHAPE_PARSED_WEIGHT} bytes"
    );
    if stops_at(Stage::Input) {
        return;
    }
    let start = Instant::now();
    let schema: Value = serde_json::from_str(&text).expect("the schema text parses");
    report("parse worst case at the caps: T_parse", start);
    if stops_at(Stage::Parse) {
        return;
    }
    let start = Instant::now();
    let copy = schema.clone();
    report("parse worst case at the caps: T_clone", start);
    let start = Instant::now();
    let compiled = compile(&copy);
    report("parse worst case at the caps: T_compile", start);
    if stops_at(Stage::Measured) {
        return;
    }
    assert_eq!(
        counted(&schema),
        (MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS),
        "the schema sits at both caps"
    );
    assert!(compiled.is_ok(), "a schema at both caps compiles");
}

/// An object schema of `count` fields `f0000`, `f0001`, …, each `field`, with only `type` and
/// `properties` beside them: three values and two objects more than the fields hold. Copied from
/// `shape_schema_field_tree.rs` (`every_admission_profile_compiles`), since one integration test
/// cannot import another's functions.
fn object_of_fields(count: usize, field: &Value) -> Value {
    let properties: serde_json::Map<String, Value> = (0..count)
        .map(|index| (format!("f{index:04}"), field.clone()))
        .collect();
    json!({ "type": "object", "properties": properties })
}

/// `compile` of each synthetic admission profile in `MAX_SHAPE_NODES`'s doc, the fixtures of
/// `shape_schema_field_tree.rs`'s `every_admission_profile_compiles`, copied. Each fixture holds
/// exactly the values and objects its row states, and compiles. The table's first row, the stored
/// shapes, is witnessed by `stored_shape_profiles_compile` in `shape_schema_field_tree.rs`, so it
/// has no fixture here.
///
/// With the node budget, arm64 (M4 Pro), rustc 1.99, release, median of three runs: `T_compile`
/// 1.16 ms (1,024 described strings), 0.76 ms (1,024 arrays of strings), 0.73 ms (1,024 nullable
/// strings with a title, a description and an example), 0.16 ms (200 fields with 32-member enums),
/// 0.26 ms (32 fields with 256-string enums).
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn compile_admission_profiles() {
    deployed_value();
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
        assert_eq!(
            counted(&schema),
            (values, objects),
            "{name}: the fixture holds what its row states"
        );
        let start = Instant::now();
        let compiled = compile(&schema);
        report(&format!("admission profile, {name}: T_compile"), start);
        assert!(compiled.is_ok(), "{name} compiles");
    }
}

/// Spine levels above the leaves of [`spine_enum_text`]: its leaves sit at depth `VALUES_SPINE + 1`
/// and their deepest `items` schema three levels below them, at `MAX_SHAPE_DEPTH`.
const VALUES_SPINE: usize = MAX_SHAPE_DEPTH - 4;

/// The values and objects one leaf of [`spine_enum_text`] holds: itself, and at each of three
/// `items` levels a schema, its `enum` array and `MAX_MIXED_ENUM_ENTRIES` numbers; the leaf and its
/// three `items` schemas the objects.
const SPINE_LEAF: (usize, usize) = (1 + 3 * (MAX_MIXED_ENUM_ENTRIES + 2), 4);

/// The text of a schema of `VALUES_SPINE` single-property objects, each key
/// `MAX_PROPERTY_NAME_BYTES` long, over as many leaves as fit both the size cap and the node
/// budget, and how many paths it declares. Each leaf declares an `enum` of `MAX_MIXED_ENUM_ENTRIES`
/// numbers at each of three `items` levels, element facts the walk reports without narrowing, so
/// `enumerate` copies the spine into each leaf's path and three enums into its facts. Built as
/// text, never as a `Value`, so the input holds only the text.
fn spine_enum_text() -> (String, usize) {
    let numbers: Vec<String> = (0..MAX_MIXED_ENUM_ENTRIES).map(|n| n.to_string()).collect();
    let numbers = numbers.join(",");
    let leaf = format!(
        r#"{{"items":{{"enum":[{numbers}],"items":{{"enum":[{numbers}],"items":{{"enum":[{numbers}]}}}}}}}}"#
    );
    let mut open = format!(r#"{{"$schema":"{DRAFT}","type":"object","properties":{{"#);
    for level in 0..VALUES_SPINE {
        let key = format!("{level:02}{}", "k".repeat(MAX_PROPERTY_NAME_BYTES - 2));
        open.push_str(&format!(r#""{key}":{{"type":"object","properties":{{"#));
    }
    let close = "}}".repeat(VALUES_SPINE + 1);
    let per = format!(r#""l0000":{leaf},"#).len();
    let by_bytes = (MAX_SCHEMA_BYTES - open.len() - close.len()) / per;
    // The root, `$schema`, `type` and `properties`, and each spine level an object, its `type` and
    // its `properties`; the root's and each level's object and `properties` the objects.
    let frame = (4 + 3 * VALUES_SPINE, 2 + 2 * VALUES_SPINE);
    let leaves = by_bytes.min(fields_in_the_budget(frame, SPINE_LEAF));
    let properties: Vec<String> = (0..leaves)
        .map(|l| format!(r#""l{l:04}":{leaf}"#))
        .collect();
    (
        format!("{open}{}{close}", properties.join(",")),
        VALUES_SPINE + leaves,
    )
}

/// A schema builder of this case: its text and how many paths it declares.
type Fixture = fn() -> (String, usize);

/// The vocabulary's values term at its worst within the node budget, each fixture filling it
/// ([`assert_fills_the_budget`]):
///
/// - `string enums`: [`string_enum_fields_text`], fields each an `enum` of `MAX_ENUM_ENTRIES`
///   strings, so `enumerate` copies every member, each a string of its own.
/// - `spine`: [`spine_enum_text`], leaves under a spine of the longest keys, each declaring an
///   `enum` of `MAX_MIXED_ENUM_ENTRIES` numbers at three `items` levels, so `enumerate` copies the
///   spine into every leaf's path and three enums into its facts.
///
/// Times `enumerate` (`T_enum`) and prints the vocabulary's wire bytes. Under `SHAPE_BENCH_CASE`,
/// runs the fixture of that name alone.
///
/// With the node budget, arm64 (M4 Pro), rustc 1.99, release, median of three runs: string enums,
/// 47 paths, `T_enum` 0.25 ms, max RSS 7.2 MiB at `parse` and 7.9 MiB at `measured` (the vocabulary
/// holds 0.75 MiB); spine, 130 paths, `T_enum` 0.18 ms, 7.1 MiB and 8.1 MiB (1.0 MiB). On `main`
/// the structured worst case held 19.7 MiB.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn vocabulary_values_worst_case() {
    deployed_value();
    let case = std::env::var("SHAPE_BENCH_CASE").unwrap_or_default();
    let fixtures: [(&str, Fixture, (usize, usize)); 2] = [
        ("string enums", string_enum_fields_text, STRING_ENUM_FIELD),
        ("spine", spine_enum_text, SPINE_LEAF),
    ];
    for (name, build, unit) in fixtures {
        if !case.is_empty() && case != name {
            continue;
        }
        let (text, paths) = build();
        println!(
            "vocabulary values, {name}: {paths} paths, {} bytes",
            text.len()
        );
        if stops_at(Stage::Input) {
            continue;
        }
        let schema: Value = serde_json::from_str(&text).expect("the schema text parses");
        if stops_at(Stage::Parse) {
            continue;
        }
        let start = Instant::now();
        let vocabulary = enumerate(&schema, &WalkBounds::BUILD);
        report(&format!("vocabulary values, {name}: T_enum"), start);
        if stops_at(Stage::Measured) {
            continue;
        }
        let wire = serde_json::to_vec(&vocabulary).expect("serializes").len();
        println!("vocabulary values, {name}: {wire} wire bytes");
        assert!(
            text.len() <= MAX_SCHEMA_BYTES,
            "{name}: within the size cap"
        );
        assert_fills_the_budget(&schema, unit, &format!("vocabulary values, {name}"));
        assert_eq!(vocabulary.paths.len(), paths, "{name}: every declared path");
        assert!(
            vocabulary.truncated.is_empty(),
            "{name}: walked whole, never cut"
        );
        assert!(compile(&schema).is_ok(), "{name}: the schema compiles");
    }
}

/// The text of a schema of as many fields as fit the size cap, each an `enum` of
/// `MAX_MIXED_ENUM_ENTRIES` distinct one-key objects `{"a":i}`, and how many fields there are.
/// Built as text, never as a `Value`, so the input holds only the text.
fn structured_enum_fields() -> (String, usize) {
    let members: Vec<String> = (0..MAX_MIXED_ENUM_ENTRIES)
        .map(|i| format!(r#"{{"a":{i}}}"#))
        .collect();
    let field = format!(r#"{{"enum":[{}]}}"#, members.join(","));
    let head = format!(r#"{{"$schema":"{DRAFT}","type":"object","properties":{{"#);
    let per = format!(r#""f0000":{field},"#).len();
    let fields = (MAX_SCHEMA_BYTES - head.len() - 2) / per;
    let properties: Vec<String> = (0..fields)
        .map(|f| format!(r#""f{f:04}":{field}"#))
        .collect();
    (format!("{head}{}}}}}", properties.join(",")), fields)
}

/// A schema no shape can now hold: an `enum` member must be a scalar, so `compile` refuses it, and
/// the walk reads a structured member as `false`, so `enumerate` declares nothing. On `main` it was
/// the vocabulary's values term at its worst: every field listed its enum, and every member was an
/// object, so `enumerate` copied a map node per member. Times `enumerate`, which now reads the
/// schema as declaring nothing.
///
/// On `main` at `0ef10cb0d`, arm64 (M4 Pro), rustc 1.99, release, median of three runs: 878
/// fields; `T_enum` 3.4 ms (3.2–4.8); max RSS 6.9 MiB at `input`, 26.7 MiB at `parse`, 46.4 MiB at
/// `measured`, so the vocabulary held 19.7 MiB; 320,177 wire bytes.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn vocabulary_values_structured_worst_case() {
    deployed_value();
    let (text, fields) = structured_enum_fields();
    assert!(
        (MAX_SCHEMA_BYTES / 2..=MAX_SCHEMA_BYTES).contains(&text.len()),
        "schema: {} bytes",
        text.len()
    );
    println!("vocabulary values, structured: {fields} fields");
    if stops_at(Stage::Input) {
        return;
    }
    let schema: Value = serde_json::from_str(&text).expect("the schema text parses");
    if stops_at(Stage::Parse) {
        return;
    }
    let start = Instant::now();
    let vocabulary = enumerate(&schema, &WalkBounds::BUILD);
    report("vocabulary values, structured: T_enum", start);
    if stops_at(Stage::Measured) {
        return;
    }
    assert!(
        vocabulary.paths.is_empty(),
        "the walk reads a structured enum member as false, so none of the {fields} fields is a path"
    );
}

/// Spine levels above the leaves of `vocabulary_paths_worst_case`: the leaves sit at
/// `MAX_WALK_DEPTH`.
const SPINE: usize = MAX_WALK_DEPTH - 1;

/// A spine of `SPINE` single-property objects, each key `MAX_PROPERTY_NAME_BYTES` long, over
/// `leaves` short `true` fields, and how many paths it declares.
fn long_spine(leaves: usize) -> (Value, usize) {
    let properties: serde_json::Map<String, Value> = (0..leaves)
        .map(|leaf| (format!("l{leaf:04}"), json!(true)))
        .collect();
    let schema = (0..SPINE).rev().fold(
        json!({ "type": "object", "properties": properties }),
        |inner, level| {
            let key = format!("{level:02}{}", "k".repeat(MAX_PROPERTY_NAME_BYTES - 2));
            json!({ "type": "object", "properties": { key: inner } })
        },
    );
    (schema, SPINE + leaves)
}

/// The vocabulary's paths term at its worst: `MAX_WALK_PATHS` paths, every leaf under the same
/// spine of the longest keys, so `enumerate` copies the whole spine into each path. Times
/// `enumerate` and prints the vocabulary's wire bytes.
///
/// On `main` at `0ef10cb0d`, arm64 (M4 Pro), rustc 1.99, release, median of three runs: `T_enum`
/// 0.46 ms; max RSS 6.7 MiB once the schema is built, 11.2 MiB at `measured`, so the vocabulary
/// holds 4.5 MiB; 4,027,856 wire bytes.
///
/// With the node budget, arm64 (M4 Pro), rustc 1.99, release, median of three runs: 1,057 values,
/// 32 objects, within the budget; `T_enum` 0.43 ms; max RSS 6.7 MiB built and 11.2 MiB at
/// `measured`, so the vocabulary holds 4.5 MiB, unchanged from `main`: the budget does not bound
/// the paths term.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn vocabulary_paths_worst_case() {
    deployed_value();
    let (schema, paths) = long_spine(MAX_WALK_PATHS - SPINE);
    assert_eq!(paths, MAX_WALK_PATHS);
    assert_compact_size_within(&schema, 1, MAX_SCHEMA_BYTES, "schema");
    // The innermost object, its `type`, its `properties` and the leaves; each spine level an
    // object, its `type` and its `properties`. The objects are each level's and its `properties`.
    let counts = counted(&schema);
    println!(
        "vocabulary paths: {} values, {} objects",
        counts.0, counts.1
    );
    assert_eq!(
        counts,
        (3 * (SPINE + 1) + (paths - SPINE), 2 * (SPINE + 1)),
        "the spine and its leaves"
    );
    assert!(
        within_node_budget(&schema),
        "the paths worst case fits the budget"
    );
    // No parse: the schema is built in code, so the control stops here.
    if stops_at(Stage::Parse) {
        return;
    }
    let start = Instant::now();
    let vocabulary = enumerate(&schema, &WalkBounds::BUILD);
    report("vocabulary paths: T_enum", start);
    if stops_at(Stage::Measured) {
        return;
    }
    let wire = serde_json::to_vec(&vocabulary).expect("serializes").len();
    println!("vocabulary paths: {wire} wire bytes");
    assert_eq!(vocabulary.paths.len(), paths, "every path, spine included");
    assert!(vocabulary.truncated.is_empty(), "walked whole, never cut");
    compile(&schema).expect("a spine of long keys compiles");
}

/// The text of a schema whose `default` holds `default` zeros and whose `examples` hold `examples`
/// zeros: with the root, `$schema` and the two arrays, `default + examples + 4` values, the root
/// the one object. Built as text, so each schema is parsed before the timer starts.
fn zeros_text(default: usize, examples: usize) -> String {
    let zeros = |count: usize| vec!["0"; count].join(",");
    format!(
        r#"{{"$schema":"{DRAFT}","default":[{}],"examples":[{}]}}"#,
        zeros(default),
        zeros(examples)
    )
}

/// How many zeros fit in an array that a text of `empty` bytes holds empty, keeping the text
/// within `MAX_SCHEMA_BYTES`: each zero adds itself and a comma, the first no comma.
fn zeros_that_fit(empty: usize) -> usize {
    (MAX_SCHEMA_BYTES - empty).div_ceil(2)
}

/// What it costs to refuse a schema past the node budget (`T_refuse`). It times
/// `within_node_budget`, the check the loader's `judge` makes before it takes a permit or copies
/// the schema: the substrate cannot call `judge`, which lives in temper-services. Each schema is
/// parsed from text before the timer starts.
///
/// - `just over`: `examples` hold `MAX_SHAPE_NODES - 3` zeros and `default` is empty, so with the
///   root, `$schema` and the two arrays the schema is one value past the node cap.
/// - `past the stop`: the same `examples`, and `default` holds as many zeros as fit
///   `MAX_SCHEMA_BYTES`. The count takes the last container it pushed first, and `examples` sorts
///   after `default`, so it reads `examples` first, passes the cap there and stops without reading
///   `default`. Its time equal to `just over`'s witnesses that the count stops at the first
///   container past a cap, however much lies beyond it.
/// - `one array`: `default` empty and `examples` holding as many zeros as fit `MAX_SCHEMA_BYTES`.
///   The count adds a container's members, reading each to count its objects, before it checks the
///   caps, so it reads this whole array: the widest single read a refusal makes.
///
/// With the node budget, arm64 (M4 Pro), rustc 1.99, release, median of three runs: `T_refuse`
/// 0.005 ms just over the budget and 0.004 ms with 256 KiB past the stop, so the count stops where
/// the budget is passed; 0.03–0.05 ms when the excess is one array, which the count reads whole
/// (one container's width) before it checks.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn refuse_past_the_node_budget() {
    deployed_value();
    let just_over = MAX_SHAPE_NODES - 3;
    let fill = zeros_that_fit(zeros_text(0, just_over).len());
    let one_array = zeros_that_fit(zeros_text(0, 0).len());
    for (name, default, examples) in [
        ("just over", 0, just_over),
        ("past the stop", fill, just_over),
        ("one array", 0, one_array),
    ] {
        let text = zeros_text(default, examples);
        assert!(
            text.len() <= MAX_SCHEMA_BYTES,
            "{name}: within the size cap"
        );
        let schema: Value = serde_json::from_str(&text).expect("the schema text parses");
        let start = Instant::now();
        let within = within_node_budget(&schema);
        report(
            &format!("refuse past the node budget, {name}: T_refuse"),
            start,
        );
        assert!(!within, "{name}: past the node budget");
        let stopped = count_up_to(&schema, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS);
        let (nodes, objects) = counted(&schema);
        println!(
            "refuse past the node budget, {name}: {} bytes, {nodes} values, {objects} objects; \
             the count stops at {} values",
            text.len(),
            stopped.nodes
        );
        assert_eq!(
            (nodes, objects),
            (default + examples + 4, 1),
            "{name}: the zeros, the root, `$schema` and the two arrays"
        );
        assert_eq!(
            stopped.nodes,
            examples + 4,
            "{name}: the count stops once it has read `examples`"
        );
    }
}
