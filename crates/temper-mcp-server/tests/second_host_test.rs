//! **`the-seam-is-the-outgoing-credential`, witnessed over the full tool list.**
//!
//! A second host — built only on the tool layer's public API (`TemperMcpService`, `RelayConfig`,
//! `IdentitySeam`), mounted on rmcp's own streamable-HTTP service with no JWT edge — whose seam
//! yields nothing. Every advertised tool, relayed or pure-compute, answers the crate's one
//! not-connected refusal: a host that supplies nothing gets refusals, never answers.
//!
//! The deployed door's own tool service, mounted the same way (so no edge plants a bearer), is
//! the other half: its seam yields nothing for a request the edge never verified, and it renders
//! the same refusal, byte for byte, on every tool.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde_json::{json, Value};
use temper_mcp::{BlobDoor, IdentitySeam, OutgoingIdentity, RelayConfig, TemperMcpService};
use tower::ServiceExt;

/// The test host's seam when it holds no credential: nothing, for every request.
struct NoCredential;

impl IdentitySeam for NoCredential {
    fn outgoing_identity(&self, _parts: &http::request::Parts) -> Option<OutgoingIdentity> {
        None
    }
}

/// Every tool advertised: the blob door open, so the blob pair is listed too.
fn open_blob_door() -> BlobDoor {
    BlobDoor::Open {
        single_request_max_bytes: 1024,
    }
}

/// The second host's tool service: never reached past the seam, so the relay target is inert.
fn test_host() -> TemperMcpService {
    TemperMcpService::new(
        open_blob_door(),
        RelayConfig::new("http://127.0.0.1:9", std::time::Duration::from_secs(5)),
        Arc::new(NoCredential),
    )
}

/// The deployed door's tool service, relay configured, built by the shell's own reader.
fn deployed_door() -> TemperMcpService {
    temper_mcp_server::tool_service(
        open_blob_door(),
        temper_mcp_server::config::deployed_relay(&|key: &str| match key {
            "TEMPER_API_BASE_URL" => Some("http://127.0.0.1:9".to_string()),
            "TEMPER_MCP_SERVICE_SECRET" => Some("a-service-secret-of-length".to_string()),
            _ => None,
        }),
    )
}

/// Mount a service as a host would: rmcp's streamable-HTTP service, stateless, JSON answers.
fn mount(service: TemperMcpService) -> axum::Router {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .disable_allowed_hosts();
    let mcp = StreamableHttpService::new(
        move || Ok(service.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    axum::Router::new().nest_service("/mcp", mcp)
}

async fn post(router: &axum::Router, body: Value) -> Value {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(header::HOST, "temper.invalid")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json, text/event-stream")
                .body(Body::from(serde_json::to_vec(&body).expect("serializes")))
                .expect("request builds"),
        )
        .await
        .expect("router serves");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body reads");
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).expect("the answer is JSON")
}

async fn tool_names(router: &axum::Router) -> Vec<String> {
    let listed = post(
        router,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}),
    )
    .await;
    listed["result"]["tools"]
        .as_array()
        .expect("tools/list answers a tool array")
        .iter()
        .map(|t| t["name"].as_str().expect("a tool has a name").to_string())
        .collect()
}

async fn call(router: &axum::Router, name: &str) -> Value {
    post(
        router,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": name, "arguments": {}}
        }),
    )
    .await
}

/// FAILS IF: any advertised tool — `describe_schema` (pure compute) included — answers anything
/// but the not-connected refusal when the seam yields no identity, on either host; or the two
/// hosts render it differently.
#[tokio::test]
async fn a_host_that_supplies_nothing_gets_the_not_connected_refusal_on_every_tool() {
    let second = mount(test_host());
    let deployed = mount(deployed_door());

    let names = tool_names(&second).await;
    for must in [
        "describe_schema",
        "blob_read",
        "blob_manage",
        "create_resource",
        "search",
    ] {
        assert!(
            names.iter().any(|n| n == must),
            "with the blob door open every registered tool is advertised; `{must}` is missing"
        );
    }
    assert_eq!(
        names,
        tool_names(&deployed).await,
        "both hosts advertise one tool list"
    );

    let expected = json!({
        "code": -32600,
        "message": temper_mcp::seam::NOT_CONNECTED_SENTENCE,
    });
    for name in &names {
        let from_second = call(&second, name).await;
        assert_eq!(
            from_second["error"], expected,
            "`{name}` answered something other than the not-connected refusal: {from_second}"
        );
        let from_deployed = call(&deployed, name).await;
        assert_eq!(
            serde_json::to_vec(&from_deployed).unwrap(),
            serde_json::to_vec(&from_second).unwrap(),
            "`{name}` renders the refusal differently on the deployed door"
        );
    }
}
