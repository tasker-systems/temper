//! The deployed door's wire witnesses: its `tools/list` answer is the tool layer's shipped
//! declarations (`declarations-are-fixtures`), and `initialize` negotiates the protocol version
//! it always has.
//!
//! First written for the rmcp 1.8 → 3.4.1 upgrade, whose beat was scoped to change nothing on
//! the wire. The declaration comparison now runs through `temper_mcp::declarations`, the helper
//! every host calls with its own bytes; the fixture and its regen live in temperkb-mcp.

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use tower::ServiceExt;

mod common;

/// Signs an HS256 bearer for the distinct-audiences fixture, whose MCP audience the gate
/// accepts. Same claims shape as `auth_surface_test`'s mint helper — duplicated here so the
/// witness file carries its own key material and cannot drift out of sync with a helper's
/// audience change mid-upgrade.
fn mcp_bearer() -> String {
    #[derive(serde::Serialize)]
    struct Claims {
        sub: &'static str,
        iss: &'static str,
        aud: &'static str,
        exp: i64,
        iat: i64,
    }
    encode(
        &Header::new(Algorithm::HS256),
        &Claims {
            sub: "auth|declaration-witness",
            iss: "https://as.test",
            aud: "https://inst.test/mcp",
            exp: i64::MAX / 2,
            iat: 0,
        },
        &EncodingKey::from_secret(b"witness"),
    )
    .expect("token signs")
}

/// POSTs one JSON-RPC request at `/mcp` with the headers a real client sends, and returns
/// the status plus the RAW body (no re-serialization — the bytes are the wire).
async fn post_mcp(request_body: Value) -> (StatusCode, Vec<u8>) {
    let router = common::build_router(
        common::state_with_distinct_audiences(),
        common::discovery_config(),
    );
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(header::HOST, "temper.invalid")
                .header(header::AUTHORIZATION, format!("Bearer {}", mcp_bearer()))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json, text/event-stream")
                .body(Body::from(
                    serde_json::to_vec(&request_body).expect("serializes"),
                ))
                .expect("request builds"),
        )
        .await
        .expect("router serves");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body reads")
        .to_vec();
    (status, bytes)
}

async fn initialize_with(protocol_version: &str) -> (StatusCode, Value) {
    let (status, bytes) = post_mcp(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": protocol_version,
            "capabilities": {},
            "clientInfo": { "name": "declaration-witness", "version": "0.0.0" }
        }
    }))
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "initialize must reach the protocol layer; body: {}",
        String::from_utf8_lossy(&bytes)
    );
    let body: Value = serde_json::from_slice(&bytes).expect("initialize response is JSON");
    (status, body)
}

/// The tool declaration set the deployed door's clients receive is the one temperkb-mcp ships.
///
/// Checks the raw `tools/list` response: the JSON-RPC envelope and, in canonical form, the
/// result. This router's fixture state closes the blob door (`blob: None`), so the helper holds
/// it to the shipped set without the blob pair, exactly as a closed-door client sees it; the
/// full-router floor is separately witnessed by `both_blob_doors_are_advertised_by_the_router`.
#[tokio::test]
async fn the_deployed_door_advertises_the_shipped_declarations() {
    let (_, bytes) = post_mcp(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    }))
    .await;
    assert!(
        serde_json::from_slice::<Value>(&bytes).is_ok(),
        "tools/list must answer JSON; got {} bytes",
        bytes.len()
    );
    // The declarations are the tool layer's (temperkb-mcp ships them as a fixture); this host
    // asserts its own wire answer against them through the crate's public helper, exactly as
    // any other host does. The fixture is regenerated in the crate (its `declarations_test`).
    temper_mcp::declarations::assert_tools_list_response(&bytes);
    let envelope: Value = serde_json::from_slice(&bytes).expect("tools/list answers JSON");
    assert_eq!(envelope["id"], 2, "the response answers this request's id");
}

/// The wire rule (strategy spec 2026-09-25): a temper MCP tool declaration is
/// self-contained — no `$ref`, no `$defs`, anywhere in any served `inputSchema`, ever.
/// Enforced globally over every served tool, never per-tool and never by memory: a
/// `$ref`-carrying declaration removes the `type: object` signal that drives client-side
/// encoding, and an unresolved `$ref` reaches a model as `null`. Written before the
/// inline fix and held RED over the pre-fix tree — that failing run is the bite.
#[tokio::test]
async fn every_served_tool_input_schema_is_self_contained() {
    let (_, bytes) = post_mcp(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/list",
        "params": {}
    }))
    .await;
    let body: Value = serde_json::from_slice(&bytes).expect("tools/list answers JSON");
    let tools = body
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .expect("tools/list carries result.tools");
    assert!(!tools.is_empty(), "the served tool set must not be empty");

    fn collect_ref_paths(value: &Value, path: &str, hits: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let child_path = format!("{path}.{key}");
                    if key == "$ref" || key == "$defs" {
                        hits.push(child_path.clone());
                    }
                    collect_ref_paths(child, &child_path, hits);
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    collect_ref_paths(item, &format!("{path}[{i}]"), hits);
                }
            }
            _ => {}
        }
    }

    let mut offenders: Vec<String> = Vec::new();
    for tool in tools {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .expect("every tool names itself");
        if let Some(schema) = tool.get("inputSchema") {
            let mut hits = Vec::new();
            collect_ref_paths(schema, name, &mut hits);
            if !hits.is_empty() {
                offenders.push(format!("  {name}: {}", hits.join(", ")));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the wire rule is self-contained declarations — zero $ref/$defs — but {} tool \
         declaration(s) carry them:\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// Initialize negotiation is stable for every version shape a client can ask.
///
/// Written on the 1.8 tree, whose handler echoes `get_info()` (protocol version
/// `"2025-11-25"`) for every request, and held across the upgrade: under 3.4.1 the
/// two-element supported list `["2025-11-25", "2026-07-28"]` makes each of these resolve to
/// the same answer — a legacy request is echoed, an ancient request falls back to the
/// server's legacy default, and a modern request is answered with the legacy fallback
/// because 2026-07-28 carries its lifecycle in per-request metadata, not the handshake.
/// Every observable a client could have received before the upgrade is one it still receives.
#[tokio::test]
async fn initialize_negotiation_is_stable_for_every_requested_version() {
    for requested in ["2025-11-25", "2024-11-05", "2026-07-28"] {
        let (_, body) = initialize_with(requested).await;
        let answered = body
            .pointer("/result/protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                panic!("initialize response carries result.protocolVersion: {body}")
            });
        assert_eq!(
            answered, "2025-11-25",
            "a client requesting {requested} must still be answered 2025-11-25"
        );
    }
}
