//! Regression guard for issue #307 — the MCP `create_resource`/`update_resource`
//! input structs carry `#[serde(flatten)] act: ActInput` alongside the
//! `open_meta` field. This proves the flatten does NOT swallow `open_meta`
//! (it deserializes from the exact reporter payload) and that schemars still
//! advertises `open_meta` in the generated tool schema, so an MCP client sends
//! it and the surface receives it.

use temper_mcp::tools::resources::{CreateResourceInput, UpdateResourceInput};

#[test]
fn create_input_deserializes_open_meta_alongside_flattened_act() {
    let json = r#"{
        "doc_type_name": "research",
        "title": "ZZ open_meta probe (delete me)",
        "context_ref": "+my-team/my-context",
        "content": "Probe body.",
        "managed_meta": {"temper-provenance": "llm-discovered"},
        "open_meta": {"marker": "TEST", "sub_marker": "999", "is_dropping": true},
        "confidence": "confident",
        "reasoning": "probe",
        "model": "claude-opus-4-8"
    }"#;

    let input: CreateResourceInput = serde_json::from_str(json).expect("deserialize input");
    assert_eq!(
        input.open_meta,
        Some(serde_json::json!({"marker": "TEST", "sub_marker": "999", "is_dropping": true})),
        "open_meta must survive the flattened `act` field"
    );

    // schemars must advertise open_meta so the MCP client actually sends it.
    let schema = serde_json::to_value(schemars::schema_for!(CreateResourceInput)).unwrap();
    let open_meta_schema = schema
        .pointer("/properties/open_meta")
        .expect("generated schema must advertise open_meta as a top-level property");

    // ...and it must be typed as an OBJECT. Left untyped (the default schema for
    // `serde_json::Value`), some MCP clients serialize the object as a JSON
    // *string* (`"{}"`), which the server rejects with `not of type "object"`.
    // `type: object` is the load-bearing part of the fix.
    assert_eq!(
        open_meta_schema.pointer("/type").and_then(|t| t.as_str()),
        Some("object"),
        "open_meta must be advertised as type=object so clients send an object, not a string: {open_meta_schema}"
    );
    // The tier stays free-form — additionalProperties must not be false.
    assert_ne!(
        open_meta_schema.pointer("/additionalProperties"),
        Some(&serde_json::Value::Bool(false)),
        "open_meta must keep additionalProperties open (free-form tier)"
    );
    // Recognized keys are advertised as hints from the canonical schema.
    assert!(
        open_meta_schema.pointer("/properties/tags").is_some(),
        "open_meta should advertise recognized convention keys (e.g. tags)"
    );
}

/// The `schema_with` override must apply to the required `open_meta` on
/// `update_resource_meta` too — that is the exact input whose stringified `"{}"`
/// surfaced the bug.
#[test]
fn update_meta_input_advertises_open_meta_as_object() {
    use temper_mcp::tools::resources::UpdateResourceMetaInput;
    let schema = serde_json::to_value(schemars::schema_for!(UpdateResourceMetaInput)).unwrap();
    assert_eq!(
        schema
            .pointer("/properties/open_meta/type")
            .and_then(|t| t.as_str()),
        Some("object"),
        "update_resource_meta open_meta must be advertised as type=object: {schema}"
    );
}

#[test]
fn update_input_deserializes_open_meta_alongside_flattened_act() {
    let json = r#"{
        "id": "019f4000-0000-7000-8000-000000000000",
        "open_meta": {"reviewed_by": "qa"},
        "confidence": "confident"
    }"#;
    let input: UpdateResourceInput = serde_json::from_str(json).expect("deserialize update input");
    assert_eq!(
        input.open_meta,
        Some(serde_json::json!({"reviewed_by": "qa"})),
        "open_meta must survive the flattened `act` field on update too"
    );
}
