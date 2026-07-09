//! The segmented-ingest tool inputs are LLM-facing: their JSON schemas are what an agent reads to
//! decide how to call them. These tests pin the shape, not the behavior.

use temper_mcp::tools::ingest::{
    IngestAppendInput, IngestBeginInput, IngestBlocksInput, IngestFinalizeInput,
};

const RESOURCE_REF: &str = "doc-019f4498-a5e1-7383-96f7-c8362b0e8daa";

#[test]
fn append_input_accepts_a_caller_with_no_chunker() {
    // The defining shape of this surface: no chunks_packed field at all. An agent cannot produce
    // one, and must not be asked to.
    let json =
        format!(r#"{{"resource":"{RESOURCE_REF}","seq":1,"content":"beta","content_hash":"abc"}}"#);
    let input: IngestAppendInput = serde_json::from_str(&json).unwrap();
    assert_eq!(input.seq, 1);
    assert_eq!(input.content, "beta");
    assert_eq!(input.content_hash, "abc");
}

#[test]
fn begin_input_carries_the_segment_budget_fields_and_the_create_fields() {
    // r##…##: the JSON body contains `"#`, which would close an r#"…"# literal.
    let json = r##"{
        "context_ref":"@me/temper",
        "doc_type_name":"research",
        "title":"Big",
        "content":"# T\n\nalpha",
        "content_hash":"abc",
        "block_budget":262144,
        "total_blocks_hint":3
    }"##;
    let input: IngestBeginInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.block_budget, Some(262_144));
    assert_eq!(input.total_blocks_hint, Some(3));
    assert!(
        input.source_hash.is_none(),
        "an agent composing in-context has no stable source identity"
    );
    // The flattened create fields survive.
    assert_eq!(input.create.title, "Big");
    assert_eq!(input.create.doc_type_name, "research");
    assert_eq!(input.create.context_ref.as_deref(), Some("@me/temper"));
}

#[test]
fn finalize_input_requires_the_echoed_body_hash() {
    let json = format!(
        r#"{{"resource":"{RESOURCE_REF}","expected_blocks":3,"expected_body_hash":"sha256:deadbeef"}}"#
    );
    let input: IngestFinalizeInput = serde_json::from_str(&json).unwrap();
    assert_eq!(input.expected_blocks, 3);
    assert_eq!(input.expected_body_hash, "sha256:deadbeef");
}

#[test]
fn blocks_input_takes_only_a_resource_ref() {
    let json = format!(r#"{{"resource":"{RESOURCE_REF}"}}"#);
    let input: IngestBlocksInput = serde_json::from_str(&json).unwrap();
    assert_eq!(input.resource, RESOURCE_REF);
}

// The tool schema is the agent's whole interface. If `chunks_packed` ever leaks into the append
// schema, an LLM will try to synthesize a base64 MessagePack blob of 768-dim vectors.
#[test]
fn append_schema_does_not_mention_chunks_packed() {
    let schema = serde_json::to_string(&schemars::schema_for!(IngestAppendInput)).unwrap();
    assert!(
        !schema.contains("chunks_packed"),
        "the agent-facing append schema must not expose chunks_packed: {schema}"
    );
    assert!(
        schema.contains("content_hash"),
        "but it does ask for the segment hash"
    );
}
