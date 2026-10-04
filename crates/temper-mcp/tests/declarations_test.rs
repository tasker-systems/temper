//! `declarations-are-fixtures`, from the tool layer's side: the service a host mounts, driven
//! through real rmcp dispatch over an in-memory transport, advertises exactly the shipped
//! [`temper_mcp::declarations::TOOLS_LIST`], with the blob door open and closed.
//!
//! The regen path also lives here (the crate's source reads no environment):
//! `UPDATE_MCP_DECLARATIONS=1 cargo test -p temperkb-mcp --test declarations_test` rewrites the
//! fixture from the open-door answer and then fails, so a regen always costs a second run that
//! must pass.

use serde_json::Value;
use temper_mcp::declarations::{assert_tools_list, canonical_json};
use temper_mcp::{BlobDoor, TemperMcpService};

/// The `tools/list` result a client receives from a service with this blob door.
async fn tools_list(blob_door: BlobDoor) -> Value {
    let service = TemperMcpService::unavailable(blob_door, "this test host relays nowhere");
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
    let advertised = tools_list(open()).await;

    if std::env::var("UPDATE_MCP_DECLARATIONS").is_ok() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/declarations/tools_list.json");
        std::fs::write(&path, canonical_json(&advertised)).expect("fixture writes");
        panic!(
            "fixture regenerated at {}; run again WITHOUT UPDATE_MCP_DECLARATIONS to assert it",
            path.display()
        );
    }

    assert_tools_list(&advertised);
}

#[tokio::test]
async fn a_closed_blob_door_advertises_the_shipped_declarations_without_the_blob_pair() {
    let advertised = tools_list(BlobDoor::Closed {
        refusal: "no blob store".into(),
    })
    .await;
    let names: Vec<&str> = advertised["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(!names.contains(&"blob_read") && !names.contains(&"blob_manage"));
    assert_tools_list(&advertised);
}
