//! `shape_schema::compile` holds each `pattern` to `MAX_PATTERN_BYTES`: a pattern that compiles
//! under the regex engine's own 10 MiB default but not under the 10 KiB bound makes its schema
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
/// the whole is several megabytes: past `MAX_PATTERN_BYTES` (10 KiB), within the engine's 10 MiB default.
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
    let error =
        compile(&schema).expect_err("a pattern past the bound makes the schema uncompilable");
    assert_eq!(
        error.to_string(),
        ShapeSchemaUncompilable::default().to_string()
    );
    assert_eq!(
        error.refused(),
        Some(
            format!(
                "pattern past the cap of {} KiB compiled, or not a regex, at /properties/code",
                MAX_PATTERN_BYTES / 1024
            )
            .as_str()
        ),
        "the refusal names the cap and the schema position holding the pattern"
    );
}

/// Control: an ordinary field pattern compiles, and is applied.
#[test]
fn an_ordinary_pattern_still_compiles() {
    let validator = compile(&with_pattern("^[A-Z]{3}-[0-9]{4}$")).expect("an ordinary pattern");
    assert!(validator.is_valid(&json!({ "code": "ABC-1234" })));
    assert!(!validator.is_valid(&json!({ "code": "abc" })));
}

/// Control: a pattern well under the bound compiles and is applied. Pins that the bound is a size
/// and not a length: a 55-fold class with a literal tail compiles to about 8.7 KB, just under 10 KiB.
#[test]
fn a_pattern_just_under_the_bound_compiles() {
    let validator = compile(&with_pattern(r"^[\w.-]{1,55}x1$")).expect("a pattern under the bound");
    assert!(validator.is_valid(&json!({ "code": "ab.-x1" })));
    assert!(!validator.is_valid(&json!({ "code": "ab" })));
}
