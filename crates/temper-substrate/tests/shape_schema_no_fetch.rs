//! `shape_schema::compile` never fetches — for `$ref` and `$schema` alike — and never echoes what a
//! fetch would have returned (task `01a15661-8460-739d-9ed3-f1e8ccbf9b69`).
//!
//! A shape's schema is written by any author of its home, so every compile of it is a fetch
//! trigger unless the compiler is configured not to. `compile` now meets the field-tree walk first
//! (spec `2026-10-27-a-shape-is-a-field-tree-design`, §1), which refuses every `$ref`, every
//! `$schema` but the bundled 2020-12 spelling at the root, and every keyword outside the field-tree
//! vocabulary — so the three connection and sentinel tests below now witness that the walk refuses
//! before any fetch could be made, not the retriever. The retriever and the compiled-out resolvers
//! stay as defence in depth; no field tree can reach them. Their assertions are unchanged and still
//! fail against `jsonschema::validator_for` with the resolver features enabled (default options
//! resolve `http(s)` and `file:`):
//!
//! - the connection count is read from a real loopback listener, so "nothing was fetched" is
//!   observed rather than inferred from an error;
//! - the `file:` witness compiles under the defaults and is refused here;
//! - the sentinel the file carries must appear in no error text.
//!
//! The rest are controls or property pins. A local `#/$defs` or `#/definitions` ref and a recursive
//! pointer `$ref` are refused as outside the field tree; a custom or off-list `$schema` is refused
//! wherever it sits as a schema keyword, including one the schema defines for itself; and a custom
//! `$schema` string inside data (`examples`, which may hold structured values) is not a schema
//! keyword and compiles. Controls pin that a standard `$schema` inside `examples`, a scalar `const`
//! holding that URI string (enforced), and a property named `$schema` compile.
//!
//! These are plain `#[test]`s on purpose: the blocking fetch a default compile makes panics inside
//! a tokio runtime instead of connecting, so the async suites' zero-connection assertions are not
//! fetch witnesses. This file and the tripwire are.
//!
//! Needs no database: runs in the Unit CI job.

#[path = "common/fetch_probe.rs"]
mod fetch_probe;

use fetch_probe::FetchProbe;
use serde_json::json;
use temper_substrate::shape_schema::{compile, ShapeSchemaUncompilable};

const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// A remote `$ref` is not followed: the compile is refused and the listener the `$ref` names sees
/// no connection. Against `validator_for` the count is at least one.
#[test]
fn a_remote_ref_is_refused_and_never_connects() {
    let probe = FetchProbe::bind();
    let schema = json!({ "$ref": probe.url("x.json") });

    assert!(
        compile(&schema).is_err(),
        "a $ref that leaves the document must not compile"
    );
    assert_eq!(
        probe.connections(),
        0,
        "the compile must not connect to the host a $ref names"
    );
}

/// A `$schema` outside the standard drafts is a meta-schema to fetch; it is refused unfetched.
/// Against `validator_for` the count is at least one.
#[test]
fn a_remote_schema_uri_is_refused_and_never_connects() {
    let probe = FetchProbe::bind();
    let schema = json!({ "$schema": probe.url("meta.json"), "type": "object" });

    assert!(
        compile(&schema).is_err(),
        "a custom meta-schema must not compile"
    );
    assert_eq!(
        probe.connections(),
        0,
        "the compile must not connect to the host a $schema names"
    );
}

/// A `file:` `$ref` is not read. The file is real and parses as a schema, so the defaults compile
/// this schema; the constructor refuses it. The sentinel the file carries appears in no error text.
#[test]
fn a_file_ref_is_refused_and_its_content_never_surfaces() {
    const SENTINEL: &str = "SENTINEL-4f7c2a91-file-content";

    let dir = std::env::temp_dir().join(format!("shape-schema-no-fetch-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("referenced.json");
    std::fs::write(
        &path,
        format!(r#"{{ "type": "string", "description": "{SENTINEL}" }}"#),
    )
    .expect("write the referenced schema");
    assert!(
        std::fs::read_to_string(&path)
            .expect("the file is readable")
            .contains(SENTINEL),
        "control: the sentinel is in the file a $ref would read"
    );

    let schema = json!({ "$ref": format!("file://{}", path.display()) });
    let outcome = compile(&schema);
    std::fs::remove_dir_all(&dir).ok();

    let error = outcome.expect_err("a file: $ref must not compile, whatever the file holds");
    assert!(
        !error.to_string().contains(SENTINEL),
        "the file's content must not appear in the error's Display"
    );
    assert!(
        !format!("{error:?}").contains(SENTINEL),
        "the file's content must not appear in the error's Debug"
    );
}

/// The error says nothing about where the `$ref` pointed: no host, no port.
#[test]
fn the_error_never_names_the_uri_it_refused() {
    let probe = FetchProbe::bind();
    let schema = json!({ "$ref": probe.url("x.json") });

    let error = compile(&schema).expect_err("refused");
    for text in [error.to_string(), format!("{error:?}")] {
        assert!(
            !text.contains("127.0.0.1") && !text.contains(&probe.port().to_string()),
            "the refusal must not echo the URI, got: {text}"
        );
    }
}

/// Every cause of an uncompilable schema answers the same value, so the text cannot be used to
/// tell a remote host's behaviour from a malformed schema.
#[test]
fn every_cause_answers_the_one_fixed_error() {
    let probe = FetchProbe::bind();
    let causes = [
        json!({ "$ref": probe.url("x.json") }),
        json!({ "$schema": "https://example.invalid/nope" }),
        json!({ "type": 12 }),
    ];
    for schema in causes {
        assert_eq!(
            compile(&schema).err(),
            Some(ShapeSchemaUncompilable),
            "schema {schema} must answer the fixed error"
        );
    }
}

/// A 2020-12 `$schema` with local `#/$defs` refs is refused: `$ref` and `$defs` are outside the
/// field tree, even though the ref never leaves the document.
#[test]
fn a_local_defs_ref_is_refused_as_outside_the_field_tree() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": { "name": { "$ref": "#/$defs/label" } },
        "$defs": { "label": { "type": "string", "minLength": 1 } },
    });

    assert!(
        compile(&schema).is_err(),
        "a local $ref is outside the field tree and must not compile"
    );
}

/// A draft-07 schema whose `$ref` reads `#/definitions/…` is refused: the draft-07 `$schema` is
/// not the accepted spelling, and `$ref` and `definitions` are outside the field tree anyway.
#[test]
fn a_draft_07_definitions_ref_is_refused() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": { "name": { "$ref": "#/definitions/label" } },
        "definitions": { "label": { "type": "string", "minLength": 1 } },
    });

    assert!(
        compile(&schema).is_err(),
        "a draft-07 schema with a definitions $ref must not compile"
    );
}

/// A `$schema` the schema defines for itself is still a custom meta-schema, and is refused. The
/// schema registers `urn:example:m` as its own `$id`, so `jsonschema`'s own check ("is the
/// meta-schema one of the document's resources?") passes and the root compiles under a guessed
/// draft; `compile` refuses it first.
#[test]
fn a_custom_schema_uri_the_schema_registers_for_itself_is_refused() {
    let schema = json!({
        "$id": "urn:example:m",
        "$schema": "urn:example:m",
        "type": "object",
    });

    assert!(
        compile(&schema).is_err(),
        "a self-registered custom meta-schema must not compile under a guessed draft"
    );
}

/// The same, with the meta-schema's URI carried by an embedded `$defs` subschema instead of the
/// root.
#[test]
fn a_custom_schema_uri_an_embedded_subschema_registers_is_refused() {
    let schema = json!({
        "$schema": "urn:example:m",
        "type": "object",
        "$defs": { "meta": { "$id": "urn:example:m", "type": "object" } },
    });

    assert!(
        compile(&schema).is_err(),
        "a meta-schema registered by an embedded $defs subschema must not compile"
    );
}

/// An embedded subschema that names a custom `$schema` of its own is refused although the root
/// names a standard draft: the subschema is a resource of its own and would compile under a guessed
/// draft once it registers itself.
#[test]
fn an_embedded_subschema_naming_a_custom_schema_uri_is_refused() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": {
            "inner": {
                "$id": "urn:example:inner",
                "$schema": "urn:example:inner",
                "type": "string",
            },
        },
    });

    assert!(
        compile(&schema).is_err(),
        "an embedded resource with a custom $schema must not compile"
    );
}

/// A custom `$schema` in a subschema under `dependencies` is refused: the document-wide rule
/// reaches it wherever it sits, including under a 2020-12 root, which has no `dependencies`
/// keyword of its own.
#[test]
fn a_custom_schema_uri_under_dependencies_is_refused() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "dependencies": { "a": { "$schema": "urn:example:m", "type": "string" } },
    });

    assert!(
        compile(&schema).is_err(),
        "a custom $schema under `dependencies` must not compile"
    );
}

/// The same for a subschema under `additionalItems`, a position the 2020-12 root does not
/// define either.
#[test]
fn a_custom_schema_uri_under_additional_items_is_refused() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "additionalItems": {
            "$id": "urn:example:m",
            "$schema": "urn:example:m",
            "type": "string",
        },
    });

    assert!(
        compile(&schema).is_err(),
        "a custom $schema under `additionalItems` must not compile"
    );
}

/// A custom `$schema` below a subschema that names a different draft than the root is refused:
/// a draft-07 root whose `definitions` hold a 2020-12 subschema, whose `$defs` hold the custom
/// `$schema`. The rule does not depend on which draft's keywords sit between the root and the
/// object that carries it.
#[test]
fn a_custom_schema_uri_below_a_mixed_draft_subschema_is_refused() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": {
            "inner": {
                "$schema": DRAFT_2020_12,
                "$defs": {
                    "meta": {
                        "$id": "urn:example:m",
                        "$schema": "urn:example:m",
                        "type": "string",
                    },
                },
            },
        },
    });

    assert!(
        compile(&schema).is_err(),
        "a custom $schema below a mixed-draft subschema must not compile"
    );
}

/// A custom `$schema` below the target of a same-document pointer `$ref` into a non-keyword
/// position (`x-custom`) is refused: the document-wide rule reaches it wherever it sits.
#[test]
fn a_custom_schema_uri_below_a_pointer_ref_target_is_refused() {
    let schema = json!({
        "$ref": "#/x-custom",
        "x-custom": { "properties": { "p": { "$schema": "urn:custom", "type": "string" } } },
    });

    assert!(
        compile(&schema).is_err(),
        "a custom $schema below a $ref pointer target must not compile"
    );
}

/// A custom `$schema` on the pointer `$ref` target itself is refused.
#[test]
fn a_custom_schema_uri_on_a_pointer_ref_target_is_refused() {
    let schema = json!({
        "$ref": "#/x-custom",
        "x-custom": { "$schema": "urn:custom", "type": "string" },
    });

    assert!(
        compile(&schema).is_err(),
        "a custom $schema on a $ref pointer target must not compile"
    );
}

/// A custom `$schema` under the `x-custom` of an embedded `$id` subschema whose `$ref` is
/// `#/x-custom` is refused, wherever the pointer would resolve.
#[test]
fn a_pointer_ref_inside_an_embedded_resource_is_resolved_against_it() {
    let schema = json!({
        "$defs": {
            "r": {
                "$id": "urn:example:r",
                "$ref": "#/x-custom",
                "x-custom": { "$schema": "urn:custom", "type": "string" },
            },
        },
    });

    assert!(
        compile(&schema).is_err(),
        "a custom $schema inside an embedded resource must not compile"
    );
}

/// A custom `$schema` under a key reached through an escaped pointer spelling is refused:
/// percent-escapes and `~0`/`~1`.
#[test]
fn a_custom_schema_uri_under_an_escaped_pointer_is_refused() {
    let escaped = [
        ("#/x%2Dcustom", "x-custom"),
        ("#/x%20custom", "x custom"),
        ("#/x~1custom", "x/custom"),
        ("#/x~0custom", "x~custom"),
    ];
    for (reference, key) in escaped {
        let schema = json!({
            "$ref": reference,
            (key): { "$schema": "urn:custom", "type": "string" },
        });
        assert!(
            compile(&schema).is_err(),
            "a custom $schema under the key {reference} names must not compile"
        );
    }
}

/// A recursive schema (a `$ref` cycle through `$defs`) is refused: a field tree has no `$ref`, so
/// nothing in it can refer back to itself.
#[test]
fn a_recursive_pointer_ref_is_refused() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "$ref": "#/$defs/node",
        "$defs": {
            "node": {
                "type": "object",
                "properties": { "next": { "$ref": "#/$defs/node" } },
            },
        },
    });

    assert!(
        compile(&schema).is_err(),
        "a recursive $ref must not compile"
    );
}

/// A custom `$schema` under the target of a reference written relative to the document root
/// (`/#/x-custom`) is refused.
#[test]
fn a_custom_schema_uri_under_a_root_relative_ref_is_refused() {
    let schema = json!({
        "$ref": "/#/x-custom",
        "x-custom": { "$schema": "urn:custom", "type": "string" },
    });

    assert!(
        compile(&schema).is_err(),
        "a custom $schema on a /#/ reference target must not compile"
    );
}

/// The same for a reference that spells out the root's default base URI (`json-schema:///`).
#[test]
fn a_custom_schema_uri_under_the_default_base_uri_ref_is_refused() {
    let schema = json!({
        "$ref": "json-schema:///#/x-custom",
        "x-custom": { "$schema": "urn:custom", "type": "string" },
    });

    assert!(
        compile(&schema).is_err(),
        "a custom $schema on a json-schema:/// reference target must not compile"
    );
}

/// The same for a reference through a resource's own `$id` (`urn:r#/x-custom`), whether the root
/// or an embedded subschema carries that `$id`.
#[test]
fn a_custom_schema_uri_under_an_id_qualified_ref_is_refused() {
    let schemas = [
        json!({
            "$id": "urn:r",
            "$ref": "urn:r#/x-custom",
            "x-custom": { "$schema": "urn:custom", "type": "string" },
        }),
        json!({
            "$ref": "urn:r#/x-custom",
            "$defs": {
                "r": {
                    "$id": "urn:r",
                    "x-custom": { "$schema": "urn:custom", "type": "string" },
                },
            },
        }),
    ];
    for schema in schemas {
        assert!(
            compile(&schema).is_err(),
            "a custom $schema on an $id-qualified reference target must not compile: {schema}"
        );
    }
}

/// The same for a reference whose pointer spells an array index loosely (`01`, `+1`), reaching an
/// array element that holds a custom `$schema`.
#[test]
fn a_custom_schema_uri_under_a_loosely_spelled_array_index_is_refused() {
    for reference in ["#/x-custom/01", "#/x-custom/+1"] {
        let schema = json!({
            "$ref": reference,
            "x-custom": [{}, { "$schema": "urn:custom", "type": "string" }],
        });
        assert!(
            compile(&schema).is_err(),
            "{reference} must not reach a custom $schema"
        );
    }
}

/// A custom `$schema` string inside an `examples` value compiles: `examples` holds data, and with
/// no `$ref` nothing can make data a schema, so the walk never reads it as a `$schema` keyword.
#[test]
fn a_custom_schema_uri_inside_an_examples_value_compiles() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "examples": [{ "$schema": "urn:x", "kind": "example" }],
    });

    assert!(
        compile(&schema).is_ok(),
        "a custom $schema inside an examples value is data and must compile"
    );
}

/// Control: an `examples` value holding a STANDARD-draft `$schema` string compiles. The rule
/// refuses only `$schema` strings no standard draft answers to.
#[test]
fn a_standard_schema_uri_inside_an_examples_value_still_compiles() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "examples": [{ "$schema": DRAFT_2020_12, "kind": "example" }],
    });

    assert!(
        compile(&schema).is_ok(),
        "an examples value holding a standard $schema is data and must compile"
    );
}

/// Control: a scalar `const` whose value is the standard `$schema` URI string compiles, and the
/// `const` is enforced.
#[test]
fn a_standard_schema_uri_inside_a_const_value_still_compiles() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "const": DRAFT_2020_12,
    });

    let validator = compile(&schema).expect("a const holding the URI string compiles");
    assert!(validator.is_valid(&json!(DRAFT_2020_12)));
    assert!(!validator.is_valid(&json!("other")));
}

/// Control: a property merely NAMED `$schema` is not a meta-schema declaration, because it is a
/// name in `properties`, not a keyword in a schema position.
#[test]
fn a_property_named_schema_is_not_mistaken_for_a_meta_schema_declaration() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": {
            "$schema": { "type": "string" },
        },
    });

    let validator = compile(&schema).expect("a property named $schema compiles");
    assert!(validator.is_valid(&json!({ "$schema": "x" })));
}

/// A custom `$schema` object nested inside an array inside an array compiles: it sits under
/// `examples`, a data position the walk never enters.
#[test]
fn a_custom_schema_uri_inside_nested_arrays_compiles() {
    let schema = json!({ "type": "object", "examples": [[{ "$schema": "urn:example:custom" }]] });

    assert!(
        compile(&schema).is_ok(),
        "a custom $schema inside an examples value's nested arrays is data and must compile"
    );
}

/// A custom `$schema` carried by the value stored under a key NAMED `$schema` is refused: the
/// walk descends into that value like any other, although the key looks like a declaration.
#[test]
fn a_custom_schema_uri_inside_a_value_under_a_schema_key_is_refused() {
    let schema = json!({
        "properties": { "$schema": { "$schema": "urn:example:custom", "type": "string" } },
    });

    assert!(
        compile(&schema).is_err(),
        "a custom $schema inside the value under a `$schema` key must not compile"
    );
}

/// Control: a `$schema` the server does not recognise is not guessed at.
#[test]
fn an_unrecognised_schema_uri_is_refused() {
    let schema = json!({ "$schema": "https://example.invalid/nope" });

    assert!(compile(&schema).is_err());
}
