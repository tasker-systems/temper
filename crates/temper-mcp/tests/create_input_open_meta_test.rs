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
    assert!(
        schema.pointer("/properties/open_meta").is_some(),
        "generated schema must advertise open_meta as a top-level property"
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
