#![cfg(feature = "test-db")]
//! Client-matrix row 3 (strategy spec 2026-09-25): a REAL rmcp 3.4.1 client's
//! typed-object calls drive both filed instances end to end.
//!
//! Row 1's finding was the encoding failure: the opencode connector stringified a
//! `$ref`-carrying object param, and the server refused the string. The declarations
//! are now self-contained, and the row-3 question is whether a conformant SDK
//! client — its own initialize handshake, tools/list, tools/call over streamable
//! HTTP — drives the formerly-`$ref`-carrying tools with REAL OBJECT arguments and
//! gets SUCCESS-voiced results. Success, not refusal: the server's own not-found and
//! plan-refusal faces both render as `invalid_params`, the same code an encoding
//! failure produces, so a refused call cannot discriminate crossing from failing.
//! Both calls here therefore land on the success path.
//!
//! The same tools/list observation doubles as the wire rule seen from the CLIENT:
//! every advertised inputSchema arrives `$ref`/`$defs`-free over the transport —
//! a distinct observation point from the in-process guard, which never leaves the
//! process.

mod common;

use jsonwebtoken::Algorithm;
use rmcp::model::{CallToolRequestParams, ClientConfig, PaginatedRequestParams};
use rmcp::service::ServiceExt;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::{json, Value};
use temper_mcp::config::McpConfig;
use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

/// The MCP router as its own server (the deployment topology), wired for the relay:
/// the harness API's base URL and the harness service credential, so `run_query`
/// executes through the network door exactly as deployed.
async fn spawn_mcp_router(pool: sqlx::PgPool, api_base_url: &str) -> std::net::SocketAddr {
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("load test RSA public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, Algorithm::RS256);

    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: common::TEST_AUDIENCE.to_string(),
            mcp_audience: common::TEST_AUDIENCE.to_string(),
            mode: AuthMode::ExternalIdp,
        },
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        mcp_service_secret: Some(common::TEST_MCP_SERVICE_SECRET.to_string()),
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
        rate_limit: None,
        blob: None,
        blob_disabled_by_policy: false,
    };

    let mcp_config = McpConfig {
        mcp_base_url: "http://mcp.test".to_string(),
        mcp_client_id: None,
        api_base_url: Some(api_base_url.to_string()),
        mcp_service_secret: Some(common::TEST_MCP_SERVICE_SECRET.to_string()),
        oauth: temper_mcp::config::OAuthStaticConfig {
            redirect_uris: vec![],
            allow_localhost: true,
        },
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mcp listener");
    let addr = listener.local_addr().expect("mcp addr");
    tokio::spawn(async move {
        axum::serve(
            listener,
            temper_mcp::build_router(AppState::new(pool, jwks_store, api_config), mcp_config),
        )
        .await
        .expect("mcp server");
    });
    addr
}

/// Every `$ref`/`$defs` occurrence anywhere under `value`, as JSON-pointer-ish paths.
fn ref_hits(value: &Value, path: &str, hits: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = format!("{path}.{key}");
                if key == "$ref" || key == "$defs" {
                    hits.push(child_path.clone());
                }
                ref_hits(child, &child_path, hits);
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                ref_hits(item, &format!("{path}[{i}]"), hits);
            }
        }
        _ => {}
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn typed_object_calls_drive_both_formerly_ref_carrying_instances_end_to_end(
    pool: sqlx::PgPool,
) {
    let app = common::setup_relay(pool).await;
    let ctx = app
        .client
        .contexts()
        .create("mcp-typed-object-e2e", None)
        .await
        .expect("context create failed");
    let created = app
        .client
        .resources()
        .create(&temper_workflow::types::resource::ResourceCreateRequest {
            kb_context_id: *ctx.id,
            doc_type: "task".to_string(),
            origin_uri: "temper://fixture/typed-object-e2e".to_string(),
            title: "Typed-object row-3 witness".to_string(),
            idempotency_key: None,
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    let mcp_addr = spawn_mcp_router(app.pool.clone(), &app.base_url()).await;
    let transport = StreamableHttpClientTransport::with_client(
        reqwest13::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(format!("http://{mcp_addr}/mcp"))
            .auth_header(app.token.clone()),
    );
    let service = ClientConfig::default()
        .serve(transport)
        .await
        .expect("the initialize handshake must succeed over the deployed transport");
    let peer = service.peer().clone();

    // The wire rule as the CLIENT sees it: not one advertised inputSchema carries
    // a `$ref` or a `$defs` over the transport.
    let tools = peer
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .expect("tools/list over the deployed transport");
    assert!(
        !tools.tools.is_empty(),
        "the deployed transport advertises the tool set"
    );
    let mut offenders: Vec<String> = Vec::new();
    for tool in &tools.tools {
        let schema = serde_json::to_value(&*tool.input_schema).expect("input schema serializes");
        let mut hits = Vec::new();
        ref_hits(&schema, tool.name.as_ref(), &mut hits);
        if !hits.is_empty() {
            offenders.push(format!("{}: {}", tool.name, hits.join(", ")));
        }
    }
    assert!(
        offenders.is_empty(),
        "every advertised inputSchema must be self-contained over the wire, but:\n{}",
        offenders.join("\n")
    );

    // Instance #1 (filed as `managed_meta` stringification): update_resource_meta with
    // a REAL OBJECT managed_meta on a resource that exists. The success face —
    // `{"updated": true, ...}` — is the discriminator: a stringified param never
    // reaches the tool body at all.
    let params = CallToolRequestParams::new("update_resource_meta".to_owned()).with_arguments(
        json!({
            "id": created.id,
            "managed_meta": {
                "temper-stage": "in-progress",
                "temper-mode": "build"
            },
            "open_meta": {}
        })
        .as_object()
        .expect("object args")
        .to_owned(),
    );
    let result = peer
        .call_tool(params)
        .await
        .unwrap_or_else(|e| panic!("update_resource_meta failed over the transport: {e:?}"));
    let text = match result.content.first() {
        Some(rmcp::model::ContentBlock::Text(t)) => t.text.clone(),
        other => panic!("no text content part: {other:?}"),
    };
    let body: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("result not JSON ({e}): {text}"));
    assert_eq!(
        body["updated"], true,
        "the object-valued managed_meta crossed and conformed: {body}"
    );

    // Instance #2 (filed as `plan` stringification): run_query with a REAL OBJECT
    // plan through the relay. An exact find for a title nothing carries returns
    // zero rows — a success envelope, not a refusal.
    let params = CallToolRequestParams::new("run_query".to_owned()).with_arguments(
        json!({
            "plan": {
                "stages": [
                    {
                        "name": "q",
                        "act": "find-exact",
                        "intention": { "query": "zz-no-such-title-row-3-probe" }
                    }
                ],
                "outcome": { "returns": [ { "stage": "q" } ] }
            }
        })
        .as_object()
        .expect("object args")
        .to_owned(),
    );
    let result = peer
        .call_tool(params)
        .await
        .unwrap_or_else(|e| panic!("run_query failed over the transport: {e:?}"));
    let text = match result.content.first() {
        Some(rmcp::model::ContentBlock::Text(t)) => t.text.clone(),
        other => panic!("no text content part: {other:?}"),
    };
    let body: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("result not JSON ({e}): {text}"));
    assert!(
        body.get("returned").map(Value::is_object).unwrap_or(false),
        "the object-valued plan crossed and executed: {body}"
    );
    let q = &body["returned"]["q"];
    assert_eq!(
        q["disposition"], "empty",
        "the exact find ran over the relay and answered an honest zero: {q}"
    );
}
