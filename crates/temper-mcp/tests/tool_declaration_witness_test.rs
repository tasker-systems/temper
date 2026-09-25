//! The MCP wire-stability witnesses for the rmcp 1.8 → 3.4.1 SDK upgrade.
//!
//! **The byte witness is the beat's attribution instrument.** The upgrade beat is scoped to
//! change NOTHING on the wire, so that the later schema-strategy beat's declaration changes
//! are attributable to that beat alone. These tests pin the raw bytes a client receives for
//! `tools/list` and the negotiated `protocolVersion` of `initialize`; if the upgrade shifts
//! either, these tests go red at the upgrade commit, not months later.
//!
//! The fixture is the raw HTTP response body — parsed-and-reserialized JSON would normalize
//! key order and license exactly the drift the witness exists to catch. Regeneration is
//! deliberately loud: `UPDATE_MCP_DECLARATIONS=1` rewrites the fixture and then FAILS, so a
//! regen can never ride silently green — it always costs a second run that must pass.

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
    let router = temper_mcp::build_router(
        common::state_with_distinct_audiences(),
        common::mcp_config(),
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
                .body(Body::from(serde_json::to_vec(&request_body).expect("serializes")))
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

/// The full tool declaration set a client receives is byte-identical across the SDK upgrade.
///
/// Pins the RAW `tools/list` response body — envelope included, since the envelope is
/// SDK-produced too and an rmcp change to it is a wire change like any other. The fixture
/// state closes the blob door (`blob: None`), so this witnesses the filtered set exactly as
/// a closed-door client sees it; the full-router floor is separately witnessed by
/// `both_blob_doors_are_advertised_by_the_router`.
#[tokio::test]
async fn tool_declarations_are_byte_identical_across_the_sdk_upgrade() {
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

    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tool_declarations_bytes.json");
    if std::env::var("UPDATE_MCP_DECLARATIONS").is_ok() {
        std::fs::create_dir_all(fixture_path.parent().unwrap()).expect("fixture dir");
        std::fs::write(&fixture_path, &bytes).expect("fixture writes");
        panic!(
            "fixture regenerated at {}; run the test again WITHOUT \
             UPDATE_MCP_DECLARATIONS to assert byte-identity against it",
            fixture_path.display()
        );
    }

    let pinned = std::fs::read(&fixture_path)
        .unwrap_or_else(|_| panic!("fixture missing at {}; regenerate with UPDATE_MCP_DECLARATIONS=1", fixture_path.display()));
    assert_eq!(
        bytes, pinned,
        "the tools/list wire bytes moved — an SDK or declaration change leaked onto the wire"
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
            .unwrap_or_else(|| panic!("initialize response carries result.protocolVersion: {body}"));
        assert_eq!(
            answered, "2025-11-25",
            "a client requesting {requested} must still be answered 2025-11-25"
        );
    }
}
