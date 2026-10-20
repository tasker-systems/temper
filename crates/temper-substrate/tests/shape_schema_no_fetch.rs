//! `shape_schema::compile` never fetches — for `$ref` and `$schema` alike — and never echoes what a
//! fetch would have returned (task `01a15661-8460-739d-9ed3-f1e8ccbf9b69`).
//!
//! A shape's schema is written by any author of its home, so every compile of it is a fetch
//! trigger unless the compiler is configured not to. The three bites are written so that they fail
//! against `jsonschema::validator_for` with the resolver features enabled (default options resolve
//! `http(s)` and `file:`); `jsonschema` is now built without those features, so to run the bite
//! re-enable `resolve-http` and `resolve-file` as well as swapping the compile:
//!
//! - the connection count is read from a real loopback listener, so "nothing was fetched" is
//!   observed rather than inferred from an error;
//! - the `file:` witness compiles under the defaults and is refused here;
//! - the sentinel the file carries must appear in no error text.
//!
//! The rest are controls or property pins, not bites. Local `#/$defs` refs still compile and an
//! unknown `$schema` is refused, pinning that the refusing retriever did not narrow what compiles.
//! Several witnesses pin that a custom `$schema` string is refused wherever it sits in the
//! document, including when the schema defines it for itself (which `jsonschema` alone would
//! compile under a guessed draft) and wherever a reference of any spelling could make it a schema.
//! One pins the accepted cost: a custom `$schema` inside a `const` value is refused too. Controls
//! pin that a standard `$schema` inside a `const`, a property named `$schema` and a recursive
//! `$ref` still compile.
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

/// Control: an explicit 2020-12 `$schema` with local `#/$defs` refs still compiles, and the
/// validator it yields applies the `$ref`. The draft's meta-schema is bundled; naming it fetches
/// nothing.
#[test]
fn a_2020_12_schema_with_local_defs_refs_still_compiles() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": { "name": { "$ref": "#/$defs/label" } },
        "$defs": { "label": { "type": "string", "minLength": 1 } },
    });

    let validator = compile(&schema).expect("a self-contained 2020-12 schema compiles");
    assert!(validator.is_valid(&json!({ "name": "ok" })));
    assert!(
        !validator.is_valid(&json!({ "name": "" })),
        "the local $ref is applied, not skipped"
    );
}

/// Control: the draft comes from `$schema`, so a draft-07 schema whose `$ref` reads
/// `#/definitions/…` compiles and applies the `$ref`.
#[test]
fn a_draft_07_schema_with_local_definitions_refs_still_compiles() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": { "name": { "$ref": "#/definitions/label" } },
        "definitions": { "label": { "type": "string", "minLength": 1 } },
    });

    let validator = compile(&schema).expect("a self-contained draft-07 schema compiles");
    assert!(validator.is_valid(&json!({ "name": "ok" })));
    assert!(!validator.is_valid(&json!({ "name": "" })));
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

/// `jsonschema` compiles `dependencies` under every draft, including a 2020-12 root whose own
/// keyword list has no `dependencies`; a custom `$schema` in one of its subschemas must still be
/// found.
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

/// The same for `additionalItems`, which is compiled under every draft too.
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

/// A subschema is compiled under ITS draft's keywords, not the root's: a draft-07 root whose
/// `definitions` hold a 2020-12 subschema, whose `$defs` hold a custom `$schema`. The draft-07
/// keyword list has no `$defs`, so a walk by the root's draft would miss it.
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

/// `jsonschema` resolves any same-document pointer and compiles what it finds as a schema, so a
/// `$ref` into a non-keyword position (`x-custom`) pulls a subschema with a custom `$schema` into
/// the compile although no draft's keyword list walks `x-custom`.
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

/// The custom `$schema` on the pointer target itself.
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

/// A pointer resolves within its enclosing resource: a `#/x-custom` inside an embedded `$id`
/// subschema names that subschema's `x-custom`, not the root's.
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
        "a pointer inside an embedded resource must be followed within that resource"
    );
}

/// The pointer is decoded before it is followed: percent-escapes and `~0`/`~1`.
#[test]
fn escaped_pointer_refs_are_followed() {
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
            "{reference} must be decoded and its target walked"
        );
    }
}

/// Control: a recursive schema (a `$ref` cycle through `$defs`) compiles and applies its `$ref`, so
/// a legitimate self-referencing schema is not refused.
#[test]
fn a_recursive_pointer_ref_schema_still_compiles() {
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

    let validator = compile(&schema).expect("a recursive schema compiles");
    assert!(validator.is_valid(&json!({ "next": { "next": {} } })));
    assert!(!validator.is_valid(&json!({ "next": 1 })));
}

/// A reference written relative to the document root (`/#/x-custom`) names the same document, so
/// its target is compiled as a schema.
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

/// The root's default base URI (`json-schema:///`) spelled out names the same document too.
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

/// A reference through a resource's own `$id` (`urn:r#/x-custom`), whether the root or an
/// embedded subschema carries that `$id`.
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

/// `referencing` parses an array index in a pointer with `str::parse::<usize>`, so `01` and `+1`
/// name element 1, although `serde_json::Value::pointer` rejects both spellings.
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

/// The accepted cost of the document-wide rule: a custom `$schema` string inside a `const` value
/// is refused too, although `const` holds data. Position cannot tell data from schema, because a
/// reference can make any position a schema, so the rule fails closed with the same fixed error.
#[test]
fn a_custom_schema_uri_inside_a_const_value_is_refused_too() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "const": { "$schema": "urn:x", "kind": "example" },
    });

    assert_eq!(
        compile(&schema).err(),
        Some(ShapeSchemaUncompilable),
        "a custom $schema inside a const is refused with the fixed error"
    );
}

/// Control: a `const` holding a STANDARD-draft `$schema` string compiles, and the `const` is
/// enforced. The rule refuses only `$schema` strings no standard draft answers to.
#[test]
fn a_standard_schema_uri_inside_a_const_value_still_compiles() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "const": { "$schema": DRAFT_2020_12, "kind": "example" },
    });

    let validator = compile(&schema).expect("a const holding a standard $schema compiles");
    assert!(validator.is_valid(&json!({ "$schema": DRAFT_2020_12, "kind": "example" })));
    assert!(!validator.is_valid(&json!({ "kind": "other" })));
}

/// Control: a property merely NAMED `$schema` is not a meta-schema declaration, because the value
/// under that key in `properties` is an object, not a string; and a standard `$schema` inside a
/// subschema is fine.
#[test]
fn a_property_named_schema_is_not_mistaken_for_a_meta_schema_declaration() {
    let schema = json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": {
            "$schema": { "type": "string" },
            "nested": { "$schema": DRAFT_2020_12, "type": "integer" },
        },
    });

    let validator = compile(&schema).expect("a property named $schema compiles");
    assert!(validator.is_valid(&json!({ "$schema": "x", "nested": 1 })));
}

/// Control: a `$schema` the server does not recognise is not guessed at.
#[test]
fn an_unrecognised_schema_uri_is_refused() {
    let schema = json!({ "$schema": "https://example.invalid/nope" });

    assert!(compile(&schema).is_err());
}
