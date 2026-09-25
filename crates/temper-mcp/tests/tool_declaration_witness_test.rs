//! The MCP wire-stability witnesses for the rmcp 1.8 → 3.4.1 SDK upgrade.
//!
//! **The byte witness is the beat's attribution instrument.** The upgrade beat is scoped to
//! change NOTHING on the wire, so that the later schema-strategy beat's declaration changes
//! are attributable to that beat alone. These tests pin the `tools/list` response in
//! canonical JSON form (every object's keys sorted — build-config independent, see
//! `canonical_json`) and the negotiated `protocolVersion` of `initialize`; if the upgrade
//! shifts either, these tests go red at the upgrade commit, not months later.
//!
//! The fixture is written-then-failed, never silently green: `UPDATE_MCP_DECLARATIONS=1`
//! rewrites it and then fails, so a regen always costs a second run that must pass.

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use std::collections::BTreeMap;
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

/// Canonical JSON: every object's keys sorted, recursively; arrays keep their order;
/// compact separators. Compared instead of the raw bytes because the raw byte order of
/// JSON OBJECT KEYS is build-config dependent, not semantic: serde_json's `preserve_order`
/// feature is unified into the build only by some workspace members, so a `-p temper-mcp`
/// run and a `--workspace` run serialize the same schema with different key order (this
/// bit CI while the fixture was raw — identical length, reordered keys). RFC 8259 leaves
/// object order insignificant, so canonicalization loses no contract; array order
/// (`required`, `enum`) IS contract and survives.
fn canonical_json(bytes: &[u8]) -> String {
    fn sort_object(value: &mut Value) {
        match value {
            Value::Object(map) => {
                let sorted: serde_json::Map<String, Value> = map
                    .iter()
                    .map(|(k, v)| {
                        let mut v = v.clone();
                        sort_object(&mut v);
                        (k.clone(), v)
                    })
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect();
                *map = sorted;
            }
            Value::Array(items) => items.iter_mut().for_each(sort_object),
            _ => {}
        }
    }
    let mut value: Value = serde_json::from_slice(bytes).expect("response parses as JSON");
    sort_object(&mut value);
    serde_json::to_string(&value).expect("canonical form serializes")
}

/// The full tool declaration set a client receives is byte-identical across the SDK upgrade.
///
/// Pins the `tools/list` response body in canonical form — envelope included, since the
/// envelope is SDK-produced too and an rmcp change to it is a wire change like any other.
/// The fixture state closes the blob door (`blob: None`), so this witnesses the filtered
/// set exactly as a closed-door client sees it; the full-router floor is separately
/// witnessed by `both_blob_doors_are_advertised_by_the_router`.
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
    let canonical = canonical_json(&bytes);

    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tool_declarations_bytes.json");
    if std::env::var("UPDATE_MCP_DECLARATIONS").is_ok() {
        std::fs::create_dir_all(fixture_path.parent().unwrap()).expect("fixture dir");
        std::fs::write(&fixture_path, canonical.as_bytes()).expect("fixture writes");
        panic!(
            "fixture regenerated at {}; run the test again WITHOUT \
             UPDATE_MCP_DECLARATIONS to assert byte-identity against it",
            fixture_path.display()
        );
    }

    let pinned = std::fs::read(&fixture_path).unwrap_or_else(|_| {
        panic!(
            "fixture missing at {}; regenerate with UPDATE_MCP_DECLARATIONS=1",
            fixture_path.display()
        )
    });
    assert_eq!(
        canonical,
        String::from_utf8(pinned).expect("fixture is UTF-8 JSON"),
        "the tools/list declaration moved — an SDK or declaration change leaked onto the wire"
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
