//! `shape_schema::compile` holds each `pattern` to `MAX_PATTERN_BYTES`: a pattern that compiles
//! under the regex engine's own 10 MiB default but not under the bound makes its schema
//! uncompilable, with the one fixed error; an ordinary pattern still compiles.
//!
//! Bite: drop `.with_pattern_options(patterns)` from `shape_schema::compile`. The large pattern
//! then compiles under the engine's default, and the refusal test goes red.
//!
//! Needs no database: runs in the Unit CI job.

use serde_json::{json, Value};
use temper_substrate::shape_schema::{compile, ShapeSchemaUncompilable, MAX_PATTERN_BYTES};

/// A large Unicode class, counted a hundred times. Each copy of `[\p{L}\p{N}]` compiles to tens of
/// kilobytes of NFA (the engine's own docs measure `\w{20}` between 300 KB and 500 KB forward), so
/// the whole is several megabytes: past `MAX_PATTERN_BYTES`, within the engine's 10 MiB default.
const LARGE_PATTERN: &str = r"[\p{L}\p{N}]{100}";

fn with_pattern(pattern: &str) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "code": { "type": "string", "pattern": pattern } }
    })
}

/// A pattern past the per-pattern bound is refused with the fixed error. The control compiles the
/// same schema with the engine's default limits (no retriever is needed: the schema names no
/// URI), which pins that the refusal is the bound's and not the pattern's.
#[test]
fn a_pattern_past_the_per_pattern_size_limit_does_not_compile() {
    let schema = with_pattern(LARGE_PATTERN);
    assert!(
        jsonschema::validator_for(&schema).is_ok(),
        "control: the pattern compiles under the engine's default limit"
    );
    assert_eq!(
        compile(&schema).err(),
        Some(ShapeSchemaUncompilable),
        "a pattern past {MAX_PATTERN_BYTES} bytes makes the schema uncompilable"
    );
}

/// Control: an ordinary field pattern compiles, and is applied.
#[test]
fn an_ordinary_pattern_still_compiles() {
    let validator = compile(&with_pattern("^[A-Z]{3}-[0-9]{4}$")).expect("an ordinary pattern");
    assert!(validator.is_valid(&json!({ "code": "ABC-1234" })));
    assert!(!validator.is_valid(&json!({ "code": "abc" })));
}
