//! A benchmark of the worst-case cost of a shape's check and compile under the caps
//! (`MAX_MIXED_ENUM_ENTRIES`, `MAX_SHAPE_PATTERNS`, `MAX_PATTERN_SOURCE_BYTES`,
//! `MAX_PATTERN_BYTES`, `MAX_SHAPE_DEPTH`, and the walk's `MAX_SCHEMA_BYTES`). Each test builds a
//! schema that sits AT the caps, then times one operation with `std::time::Instant` and prints
//! `name: <ms> ms`:
//!
//! - `check_mixed_enum_at_the_cap`: `first_failure` of a 4.5 MB array against an `enum` of
//!   `MAX_MIXED_ENUM_ENTRIES` nested objects that differ only in their last integer.
//! - `check_dfa_defeating_pattern` (and its anchored twin): `first_failure` of one 4.5 MB string
//!   of `a`/`b` against `(a|b)*a(a|b){20}`, a pattern whose DFA has a state for every window of
//!   the last 21 characters.
//! - `compile_at_the_pattern_cap`: `compile` of a schema of `MAX_SCHEMA_BYTES` holding
//!   `MAX_SHAPE_PATTERNS` distinct patterns, each compiling to just under `MAX_PATTERN_BYTES`.
//! - `check_dfa_pattern_at_the_cap`: the same a/b string against `^(a|b)*a(a|b){DFA_REPEAT}$`.
//! - `check_long_property_name_at_the_cap`: 4.5 MB of two-key objects under `items`, against a
//!   `properties` key of `MAX_PROPERTY_NAME_BYTES`.
//!
//! Run before merging a change to a cap (release, so the numbers are the deployed ones):
//!
//! ```text
//! cargo test -p temper-substrate --all-features --release --test shape_schema_cost_bench -- --ignored --nocapture
//! ```
//!
//! Peak memory is not measured here (that would need a counting allocator, which needs `unsafe`):
//! run each test alone under `/usr/bin/time -l` and read "maximum resident set size".
//!
//! Every test is `#[ignore]` and asserts nothing about time: only that the construction is what it
//! claims (the schema compiles, the check runs to a verdict). Needs no database.

use std::time::Instant;

use serde_json::{json, Value};
use temper_core::types::artifact_ask::walk::MAX_SCHEMA_BYTES;
use temper_substrate::shape_schema::{
    compile, first_failure, MAX_MIXED_ENUM_ENTRIES, MAX_PATTERN_SOURCE_BYTES,
    MAX_PROPERTY_NAME_BYTES, MAX_SHAPE_DEPTH, MAX_SHAPE_PATTERNS,
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

/// Integers in each enum entry's `v` array.
const ENTRY_INTS: usize = 16;

/// Levels of `properties` between the array's items and the schema that holds the `enum`.
const NESTING: usize = 4;

/// Milliseconds since `start`, printed as `name: <ms> ms`.
fn report(name: &str, start: Instant) {
    println!("{name}: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
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

/// An enum entry: a constant `k` and `ENTRY_INTS` integers, all equal but the last, which is
/// `last`.
fn entry(last: usize) -> Value {
    let mut ints = vec![json!(7); ENTRY_INTS - 1];
    ints.push(json!(last));
    json!({ "k": 0, "v": ints })
}

fn assert_compact_size_within(value: &Value, floor: usize, ceiling: usize, what: &str) {
    let len = serde_json::to_vec(value).expect("serializes").len();
    assert!(
        (floor..=ceiling).contains(&len),
        "{what}: {len} compact bytes, expected {floor}..={ceiling}"
    );
}

/// The root's `items` is an object schema nested `NESTING` levels deep whose innermost schema
/// holds an `enum` of `MAX_MIXED_ENUM_ENTRIES` objects (a mixed enum, scanned entry by entry). The
/// content is 4.5 MB of entries equal to the last one (each is a valid element and costs a scan of
/// the whole enum) except for the final element, whose last integer is in no entry: the check runs
/// through every element and fails there.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn check_mixed_enum_at_the_cap() {
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
    time_pattern_check("check_dfa_defeating_pattern", "(a|b)*a(a|b){20}");
}

/// The same pattern anchored at both ends, so a match must be decided at the end of the string
/// and the engine must read all 4.5 MB.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn check_dfa_defeating_pattern_anchored() {
    time_pattern_check("check_dfa_defeating_pattern_anchored", "^(a|b)*a(a|b){20}$");
}

/// `MAX_SHAPE_PATTERNS` properties, each with its own pattern `^[\w.-]{1,k}x<NN>$` (a distinct literal
/// suffix), padded with `description` text so the compact schema is `MAX_SCHEMA_BYTES`. Times
/// `compile` alone.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn compile_at_the_pattern_cap() {
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
    let pattern = format!("^(a|b)*a(a|b){{{DFA_REPEAT}}}$");
    time_pattern_check("check_dfa_pattern_at_the_cap", &pattern);
}

/// A `properties` key of `MAX_PROPERTY_NAME_BYTES` under `items`; the content is 4.5 MB of
/// `{"a":0,"b":0}` objects, so each object costs a lookup of the long key.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture before merge"]
fn check_long_property_name_at_the_cap() {
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
