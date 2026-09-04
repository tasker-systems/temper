#![cfg(feature = "test-db")]
//! S6: the S5 blob tools through a REAL rmcp client over the deployed streamable-HTTP
//! transport.
//!
//! What S5 witnessed was the tools at the function boundary (`TemperMcpService` built in-process,
//! tool fns called directly — `mcp_segmented_ingest_test`'s pattern). The declared hole was the
//! transport: does a conformant MCP CLIENT — initialize handshake, tools/list, tools/call, JSON
//! over streamable HTTP — see the same tools, and do BYTES survive the base64-in-JSON round trip
//! byte-for-byte?
//!
//! The topology mirrors the deployment (`api/mcp.rs`): the MCP router is its OWN server
//! (`temper_mcp::build_router`), separate from the API server, sharing only the database and the
//! auth configuration. The test profile is provisioned/approved through the API harness first —
//! the same standing the deployed profile would have — and the MCP surface resolves it per call
//! from the JWT claims the auth middleware injects.
//!
//! The bytes are deliberately binary-hostile: every byte value 0x00–0xFF appears (the pattern
//! covers all 256 residues), so any transport mangling — UTF-8 coercion, whitespace folding,
//! trailing-newline injection — bites here rather than in production.

mod common;

use base64::Engine as _;
use jsonwebtoken::Algorithm;
use rmcp::model::{CallToolRequestParams, ClientInfo, PaginatedRequestParams};
use rmcp::service::{ServerSink, ServiceExt};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use sha2::{Digest, Sha256};
use temper_mcp::config::{McpConfig, OAuthStaticConfig};
use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

/// Spawn the MCP router as its own server, sharing the harness pool + auth config, with the
/// blob flow live (in-memory store — the same seam `common::setup_with_blob_store` uses).
async fn spawn_mcp_server(
    pool: sqlx::PgPool,
    blob_config: temper_services::config::BlobConfig,
) -> std::net::SocketAddr {
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
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
        rate_limit: None,
        blob: Some(blob_config),
        blob_disabled_by_policy: false,
    };

    let mut api_state = AppState::new(pool, jwks_store, api_config);
    api_state.blob_store = Some(std::sync::Arc::new(
        temper_substrate::blob_store::InMemoryBlobStore::default(),
    ));

    let mcp_config = McpConfig {
        mcp_base_url: "http://mcp.test".to_string(),
        mcp_client_id: None,
        oauth: OAuthStaticConfig {
            redirect_uris: vec![],
            allow_localhost: true,
        },
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mcp listener");
    let addr = listener.local_addr().expect("mcp addr");
    tokio::spawn(async move {
        axum::serve(listener, temper_mcp::build_router(api_state, mcp_config))
            .await
            .expect("mcp server");
    });
    addr
}

/// Extract the text of the first content part of a tool result.
fn text_of(result: rmcp::model::CallToolResult) -> String {
    match result.content.first().map(|c| &c.raw) {
        Some(rmcp::model::RawContent::Text(t)) => t.text.clone(),
        other => panic!("tool returned no text content part: {other:?}"),
    }
}

async fn call_tool(
    peer: &ServerSink,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let params = CallToolRequestParams::new(name.to_owned())
        .with_arguments(arguments.as_object().expect("object args").to_owned());
    let result = peer
        .call_tool(params)
        .await
        .unwrap_or_else(|e| panic!("{name} failed over the transport: {e:?}"));
    let text = text_of(result);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name} result ({e}): {text}"))
}

/// The refusal face: a tool call the server declines arrives as a JSON-RPC error, and the
/// error message is the refusal vocabulary.
async fn call_tool_err(
    peer: &ServerSink,
    name: &str,
    arguments: serde_json::Value,
) -> rmcp::ErrorData {
    let params = CallToolRequestParams::new(name.to_owned())
        .with_arguments(arguments.as_object().expect("object args").to_owned());
    match peer.call_tool(params).await {
        Err(rmcp::service::ServiceError::McpError(e)) => e,
        other => panic!("{name} must be refused with the MCP protocol error, got: {other:?}"),
    }
}

/// A conformant client over real HTTP, with the harness token as the bearer — the setup
/// the byte-for-byte witness runs, factored for the refusal witnesses.
async fn connect_client(
    mcp_addr: std::net::SocketAddr,
    token: &str,
) -> (
    rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientInfo>,
    ServerSink,
) {
    let transport = StreamableHttpClientTransport::with_client(
        reqwest13::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(format!("http://{mcp_addr}/mcp"))
            .auth_header(token.to_string()),
    );
    let service = ClientInfo::default()
        .serve(transport)
        .await
        .expect("the initialize handshake must succeed over the deployed transport");
    let peer = service.peer().clone();
    (service, peer)
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn blob_tools_survive_the_real_transport_byte_for_byte(pool: sqlx::PgPool) {
    // Provision through the API harness first: the approved profile + a readable context,
    // exactly the standing a deployed profile arrives with.
    let app = common::setup_with_blob_store(pool).await;
    let ctx = app
        .client
        .contexts()
        .create("mcp-blob-transport-e2e", None)
        .await
        .expect("context create failed");
    let ctx_id = ctx.id.to_string();

    // The MCP router as its own deployment, blob flow live.
    let blob_config = app_blob_config();
    let mcp_addr = spawn_mcp_server(app.pool.clone(), blob_config).await;

    // A conformant client, over real HTTP, with the harness token as the bearer. The client
    // is rmcp's OWN reqwest (aliased `reqwest13` — the version its `StreamableHttpClient`
    // impl covers); `auth_header` takes the RAW token and adds the `Bearer ` prefix itself.
    // The config default is stateless-tolerant (`allow_stateless: true`), which this server
    // requires: the deployed router runs `stateful_mode(false)`.
    let transport = StreamableHttpClientTransport::with_client(
        reqwest13::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(format!("http://{mcp_addr}/mcp"))
            .auth_header(app.token.clone()),
    );
    let client_info = ClientInfo::default();
    let service = client_info
        .serve(transport)
        .await
        .expect("the initialize handshake must succeed over the deployed transport");
    let peer = service.peer().clone();

    // The S5 advertisement pair rides the deployed transport: both tools listed.
    let tools = peer
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .expect("tools/list over the deployed transport");
    let names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.contains(&"blob_read"),
        "blob_read is advertised: {names:?}"
    );
    assert!(
        names.contains(&"blob_manage"),
        "blob_manage is advertised: {names:?}"
    );

    // Hostile bytes: every value 0x00–0xFF appears; not valid UTF-8 anywhere by construction.
    let bytes: Vec<u8> = (0..256 * 1024u32)
        .map(|i| ((i * 7 + 13) % 256) as u8)
        .collect();
    let content_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    // commit — base64 in, JSON only, over the wire.
    let commit = call_tool(
        &peer,
        "blob_manage",
        serde_json::json!({
            "action": "commit",
            "home_table": "kb_contexts",
            "home_id": ctx_id,
            "content_type": "application/pdf",
            "content": content_b64,
        }),
    )
    .await;
    let blob_id = commit["blob_id"]
        .as_str()
        .unwrap_or_else(|| panic!("commit result carries blob_id: {commit}"))
        .to_string();
    assert_eq!(
        commit["deduped"], false,
        "first commit is not a dedup hit: {commit}"
    );
    // The wire's content_hash is the client-side sha256 — the cross-check that pins byte
    // integrity independent of the base64 decode below.
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let expected_hash = hex::encode(hasher.finalize());
    assert_eq!(
        commit["content_hash"], expected_hash,
        "the committed content hash is the source bytes' sha256: {commit}"
    );

    // read — base64 out; decode and compare EXACTLY.
    let read = call_tool(
        &peer,
        "blob_read",
        serde_json::json!({ "action": "read", "blob_id": blob_id }),
    )
    .await;
    assert_eq!(
        read["content_bytes"],
        bytes.len() as i64,
        "the byte count survives: {read}"
    );
    let roundtripped = base64::engine::general_purpose::STANDARD
        .decode(
            read["content_base64"]
                .as_str()
                .unwrap_or_else(|| panic!("read result carries content_base64: {read}")),
        )
        .expect("content_base64 decodes");
    assert_eq!(
        roundtripped, bytes,
        "the bytes come back byte-for-byte through the deployed transport"
    );

    // get-or-create through the wire: the same bytes again dedup to the SAME id.
    let recommit = call_tool(
        &peer,
        "blob_manage",
        serde_json::json!({
            "action": "commit",
            "home_table": "kb_contexts",
            "home_id": ctx_id,
            "content_type": "application/pdf",
            "content": content_b64,
        }),
    )
    .await;
    assert_eq!(
        recommit["blob_id"], blob_id,
        "a re-commit of identical bytes converges on the same blob: {recommit}"
    );
    assert_eq!(
        recommit["deduped"], true,
        "the re-commit reports the dedup hit: {recommit}"
    );

    // relate — the second write verb over the wire; peer is the other committed blob so the
    // whole pass stays inside the MCP surface.
    let other = call_tool(
        &peer,
        "blob_manage",
        serde_json::json!({
            "action": "commit",
            "home_table": "kb_contexts",
            "home_id": ctx_id,
            "content_type": "application/pdf",
            "content": base64::engine::general_purpose::STANDARD.encode(b"other bytes"),
        }),
    )
    .await;
    let other_id = other["blob_id"].as_str().expect("second blob_id");
    let related = call_tool(
        &peer,
        "blob_manage",
        serde_json::json!({
            "action": "relate",
            "blob_id": blob_id,
            "peer_table": "kb_blobs",
            "peer_id": other_id,
            "edge_kind": "near",
            "polarity": "forward",
            "label": "companions_to",
            "weight": 0.5,
        }),
    )
    .await;
    assert!(
        related["edge_handle"].as_str().is_some(),
        "relate ack carries the edge handle: {related}"
    );

    // list — the read-set answers over the wire: both blobs visible. The result is the row
    // array itself, no wrapper object.
    let list = call_tool(&peer, "blob_read", serde_json::json!({ "action": "list" })).await;
    let listed = list
        .as_array()
        .unwrap_or_else(|| panic!("list result is the row array: {list}"));
    let listed_ids: Vec<&str> = listed
        .iter()
        .filter_map(|b| b["blob_id"].as_str())
        .collect();
    assert!(
        listed_ids.contains(&blob_id.as_str()) && listed_ids.contains(&other_id),
        "both committed blobs are in the readable set: {listed_ids:?}"
    );

    service.cancel().await.ok();
}

/// FAILS IF: the MCP commit door ever lets over-threshold bytes through — review F5's
/// overshoot, where the threshold lived only in the HTTP multipart handler while the MCP
/// tool decoded and provider-put past it before any size decision. The gate lives in the
/// shared commit seam; this witness rides the REAL transport to pin the refusal's face:
/// invalid-params naming the threshold in force and the segmented path beyond it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_mcp_commit_door_refuses_over_threshold_bytes(pool: sqlx::PgPool) {
    let app = common::setup_with_blob_store(pool).await;
    let ctx = app
        .client
        .contexts()
        .create("mcp-blob-threshold-e2e", None)
        .await
        .expect("context create failed");

    // One MCP server with a deliberately tiny threshold — the refusal must name it.
    let mut blob_config = app_blob_config();
    blob_config.single_request_max_bytes = 1024;
    let mcp_addr = spawn_mcp_server(app.pool.clone(), blob_config).await;
    let (service, peer) = connect_client(mcp_addr, &app.token).await;

    let over_threshold = vec![9u8; 4096]; // 4× the threshold
    let err = call_tool_err(
        &peer,
        "blob_manage",
        serde_json::json!({
            "action": "commit",
            "home_table": "kb_contexts",
            "home_id": ctx.id.to_string(),
            "content_type": "application/pdf",
            "content": base64::engine::general_purpose::STANDARD.encode(&over_threshold),
        }),
    )
    .await;
    assert!(
        err.message.contains("single-request threshold"),
        "the refusal names the threshold in force: {}",
        err.message
    );
    assert!(
        err.message.contains("segmented upload path"),
        "the refusal names the path beyond the threshold: {}",
        err.message
    );

    service.cancel().await.ok();
}

/// The same D9 config the API-side harness uses, so both servers speak one policy.
fn app_blob_config() -> temper_services::config::BlobConfig {
    temper_services::config::BlobConfig {
        store_id: "test-blob-store".to_string(),
        read_write_token: None,
        credential_mode: temper_services::config::BlobCredentialMode::Token,
        oidc_token_source: std::sync::Arc::new(|| None),
        max_bytes: 100 * 1024 * 1024,
        allowlist: vec![
            "image/png".into(),
            "image/jpeg".into(),
            "image/webp".into(),
            "image/svg+xml".into(),
            "image/gif".into(),
            "application/pdf".into(),
        ],
        single_request_max_bytes: 4 * 1024 * 1024,
    }
}
