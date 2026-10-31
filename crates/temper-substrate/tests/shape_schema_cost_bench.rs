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
//!   `additionalProperties: false`, and trees of 2, 4 and 16 children a node — each timed for
//!   `compile`, a check of content holding every key, and the ask walk's `enumerate`, which no
//!   visit budget bounds. Two more are walked whole without reaching `MAX_WALK_PATHS`: fields
//!   that are each `false`, which the walk reads and never counts, and fields each carrying an
//!   enum of `MAX_ENUM_ENTRIES` strings, as many as fit.
//! - `parse_worst_case_at_the_byte_cap`: a schema of `MAX_SCHEMA_BYTES` whose `examples` hold as
//!   many `{"":0}` as fit, at the root and at the foot of an `items` chain, far past the node
//!   budget; times parsing its text, cloning the parsed `Value` and `compile`'s refusal.
//! - `vocabulary_values_structured_worst_case`: a schema no shape can now hold, walk-only: as many
//!   fields as fit, each an `enum` of `MAX_MIXED_ENUM_ENTRIES` one-key objects; times `enumerate`,
//!   which copies every member.
//! - `vocabulary_paths_worst_case`: `MAX_WALK_PATHS` paths under a spine of keys of
//!   `MAX_PROPERTY_NAME_BYTES`; times `enumerate`, which copies the spine into every path.
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
//! run each test alone under `/usr/bin/time -l` and read "maximum resident set size". The three
//! cases above stop early under `SHAPE_BENCH_STOP` ([`stops_at`]), so the same test run as a
//! control isolates what its measured step holds.
//!
//! Every test is `#[ignore]` and asserts nothing about time: only that the construction is what it
//! claims (the schema compiles or is refused, the check runs to a verdict). Needs no database.

use std::time::Instant;

use serde_json::{json, Value};
use temper_core::types::artifact_ask::walk::{MAX_SCHEMA_BYTES, MAX_WALK_DEPTH, MAX_WALK_PATHS};
use temper_core::types::artifact_ask::{enumerate, WalkBounds};
use temper_substrate::shape_schema::{
    compile, count_up_to, first_failure, within_node_budget, ShapeSchemaUncompilable,
    MAX_ENUM_ENTRIES, MAX_MIXED_ENUM_ENTRIES, MAX_PATTERN_SOURCE_BYTES, MAX_PROPERTY_NAME_BYTES,
    MAX_SHAPE_DEPTH, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS, MAX_SHAPE_PATTERNS,
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

/// An object schema of as many fields `f00000`, `f00001`, … as fit both `MAX_SCHEMA_BYTES` and the
/// node budget, each `field`; with `closed`, every field is also `required` and
/// `additionalProperties` is `false`.
fn wide_object(field: &Value, closed: bool) -> Value {
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
    schema
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
/// `MAX_SCHEMA_BYTES` and the node budget.
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
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn many_tiny_subschemas_at_the_size_cap() {
    deployed_value();
    time_at_the_size_cap("empty fields", &wide_object(&json!({}), false));
    time_at_the_size_cap("true fields", &wide_object(&json!(true), false));
    time_at_the_size_cap(
        "typed fields",
        &wide_object(&json!({ "type": "integer" }), false),
    );
    time_at_the_size_cap(
        "typed fields, closed",
        &wide_object(&json!({ "type": "integer" }), true),
    );
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
        time_at_the_size_cap(&format!("tree of {children} at depth {depth}"), &schema);
    }
    time_a_whole_walk("false fields", &wide_object(&json!(false), false), 0);
    let (schema, fields) = enum_fields();
    time_a_whole_walk("enum-carrying fields", &schema, fields);
}

/// As many fields as fit both the size cap and the node budget, each an `enum` of
/// `MAX_ENUM_ENTRIES` short distinct codes, and how many there are.
fn enum_fields() -> (Value, usize) {
    let members: Vec<Value> = (0..MAX_ENUM_ENTRIES)
        .map(|c| json!(format!("c{c:05}")))
        .collect();
    let field = json!({ "enum": members });
    let per = serde_json::to_vec(&json!({ "f0000": field }))
        .expect("serializes")
        .len();
    let by_bytes = (MAX_SCHEMA_BYTES - 256) / per;
    // The root, `$schema`, `type` and `properties`.
    let fields = by_bytes.min(fields_in_the_budget((4, 2), counted(&field)));
    let properties: serde_json::Map<String, Value> = (0..fields)
        .map(|f| (format!("f{f:04}"), field.clone()))
        .collect();
    let schema = json!({ "$schema": DRAFT, "type": "object", "properties": properties });
    (schema, fields)
}

/// Time `compile` and `enumerate` of a schema the walk reads to its end without a cut: the
/// vocabulary holds `paths` paths and was truncated at no bound.
fn time_a_whole_walk(name: &str, schema: &Value, paths: usize) {
    assert_compact_size_within(schema, 1, MAX_SCHEMA_BYTES, name);
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
/// this case is walk-only. It was the vocabulary's values term at its worst on
/// `main`: every field lists its enum, and every member is an object, so `enumerate` copies a map
/// node per member. Times `enumerate` and prints the vocabulary's wire bytes.
///
/// On `main` at `0ef10cb0d`, arm64 (M4 Pro), rustc 1.99, release, median of three runs: 878 fields; `T_enum` 3.4 ms (3.2–4.8); max RSS 6.9 MiB at
/// `input`, 26.7 MiB at `parse`, 46.4 MiB at `measured`, so the vocabulary holds 19.7 MiB; 320,177
/// wire bytes.
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
    let wire = serde_json::to_vec(&vocabulary).expect("serializes").len();
    println!("vocabulary values, structured: {wire} wire bytes");
    assert_eq!(vocabulary.paths.len(), fields, "every field is a path");
    assert!(vocabulary.truncated.is_empty(), "walked whole, never cut");
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
/// On `main` at `0ef10cb0d`, arm64 (M4 Pro), rustc 1.99, release, median of three runs: `T_enum` 0.46 ms; max RSS 6.7 MiB once the schema is built,
/// 11.2 MiB at `measured`, so the vocabulary holds 4.5 MiB; 4,027,856 wire bytes.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn vocabulary_paths_worst_case() {
    deployed_value();
    let (schema, paths) = long_spine(MAX_WALK_PATHS - SPINE);
    assert_eq!(paths, MAX_WALK_PATHS);
    assert_compact_size_within(&schema, 1, MAX_SCHEMA_BYTES, "schema");
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
