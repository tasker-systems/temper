//! `declarations-are-fixtures`, from the tool layer's side: the service a host mounts, driven
//! through real rmcp dispatch over an in-memory transport, advertises exactly the shipped
//! [`temper_mcp::declarations::TOOLS_LIST`], with the blob door open and closed.
//!
//! The regen path also lives here (the crate's source reads no environment):
//! `UPDATE_MCP_DECLARATIONS=1 cargo test -p temperkb-mcp --test declarations_test` rewrites the
//! fixture from the open-door answer and then fails, so a regen always costs a second run that
//! must pass.

use serde_json::Value;
use temper_mcp::declarations::{assert_tools_list, canonical_json, without_complete_result_type};
use temper_mcp::{BlobDoor, TemperMcpService};

/// The `tools/list` result a client receives from a service with this blob door.
async fn tools_list(blob_door: &BlobDoor) -> Value {
    let service = TemperMcpService::unavailable(blob_door.clone(), "this test host relays nowhere");
    let (server_io, client_io) = tokio::io::duplex(1 << 22);
    let server = tokio::spawn(async move {
        let running = rmcp::serve_server(service, server_io).await.expect("serve");
        let _ = running.waiting().await;
    });
    let client = rmcp::serve_client((), client_io).await.expect("client");
    let result = client.list_tools(None).await.expect("tools/list answers");
    client.cancel().await.expect("client closes");
    server.abort();
    serde_json::to_value(result).expect("the result serializes")
}

fn open() -> BlobDoor {
    BlobDoor::Open {
        single_request_max_bytes: 1024,
    }
}

#[tokio::test]
async fn the_mounted_service_advertises_the_shipped_declarations() {
    let advertised = tools_list(&open()).await;

    if std::env::var("UPDATE_MCP_DECLARATIONS").is_ok() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/declarations/tools_list.json");
        // Normalized, so a negotiated `resultType: "complete"` never lands in the fixture.
        std::fs::write(
            &path,
            canonical_json(&without_complete_result_type(&advertised)),
        )
        .expect("fixture writes");
        panic!(
            "fixture regenerated at {}; run again WITHOUT UPDATE_MCP_DECLARATIONS to assert it",
            path.display()
        );
    }

    assert_tools_list(&advertised, &open());
}

#[tokio::test]
async fn a_closed_blob_door_advertises_the_shipped_declarations_without_the_blob_pair() {
    let closed = BlobDoor::Closed {
        refusal: "no blob store".into(),
    };
    let advertised = tools_list(&closed).await;
    let names: Vec<&str> = advertised["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(!names.contains(&"blob_read") && !names.contains(&"blob_manage"));
    assert_tools_list(&advertised, &closed);
}

/// Every tool says whether it writes. A tool with no annotations reads, by MCP's defaults, as a
/// destructive write to an open world, so a client that asks the user by annotation could not
/// tell a read from a write. The rules held here:
///
/// - every tool states `readOnlyHint`, and `openWorldHint: false` (each one works inside the
///   caller's Temper instance);
/// - a write also states `destructiveHint`;
/// - a `*_read`, `get_*` or `list_*` tool is read-only, and nothing named `*_manage` is;
/// - the reads and the destructive writes are each exactly a pinned set, so moving a tool across
///   either line is a reviewed change here as well as in the fixture.
///
/// FAILS IF a tool ships without annotations, or a write's classification changes unreviewed.
#[tokio::test]
async fn every_tool_says_whether_it_writes() {
    const DESTRUCTIVE: [&str; 13] = [
        "commit_data_artifact",
        "context_manage",
        "declare_data_artifact_shape",
        "delete_resource",
        "facet_retract",
        "facet_set",
        "invitation_manage",
        "relationship",
        "resource_reblock",
        "steward_advance_watermark",
        "team_manage",
        "update_resource",
        "update_resource_meta",
    ];
    const READ_ONLY: [&str; 21] = [
        "blob_read",
        "cogmap_list",
        "cogmap_read",
        "context_read",
        "describe_schema",
        "element_trail",
        "facets_read",
        "get_block",
        "get_block_provenance",
        "get_data_artifact",
        "get_data_artifact_shape",
        "get_resource",
        "invitation_read",
        "invocation_read",
        "list_data_artifact_shapes",
        "list_data_artifacts",
        "list_resources",
        "resource_lineage",
        "run_query",
        "search",
        "steward_ingest_delta",
    ];
    let advertised = tools_list(&open()).await;
    let mut read_only_tools = Vec::new();
    let mut destructive = Vec::new();
    for tool in advertised["tools"].as_array().expect("tools") {
        let name = tool["name"].as_str().expect("name");
        let hints = &tool["annotations"];
        let read_only = hints["readOnlyHint"]
            .as_bool()
            .unwrap_or_else(|| panic!("{name} states readOnlyHint"));
        assert_eq!(hints["openWorldHint"], false, "{name}: openWorldHint");
        let named_read =
            name.ends_with("_read") || name.starts_with("get_") || name.starts_with("list_");
        if named_read {
            assert!(read_only, "{name} is named as a read");
        }
        if read_only {
            read_only_tools.push(name);
        }
        if name.ends_with("_manage") {
            assert!(!read_only, "{name} is named as a write");
        }
        if !read_only {
            let is_destructive = hints["destructiveHint"]
                .as_bool()
                .unwrap_or_else(|| panic!("{name} is a write and states destructiveHint"));
            if is_destructive {
                destructive.push(name);
            }
        }
    }
    read_only_tools.sort_unstable();
    assert_eq!(read_only_tools, READ_ONLY);
    destructive.sort_unstable();
    assert_eq!(destructive, DESTRUCTIVE);
}
