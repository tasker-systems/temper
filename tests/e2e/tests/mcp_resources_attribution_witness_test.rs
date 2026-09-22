#![cfg(feature = "test-db")]
//! The beat G3a attribution witness: an MCP-originated RESOURCES act attributes `@mcp`
//! at the ledger — through the REAL MCP server.
//!
//! What this witness adds over [G1's door witness](../../crates/temper-api/tests/
//! in_process_door_test.rs) is exactly the wiring the handoff named as the trap: G1
//! proved the transport PRIMITIVE by driving temper-client against a router directly.
//! Here a conformant MCP client runs the initialize handshake over the deployed
//! streamable-HTTP transport against `temper_mcp::build_router` — the real server, whose
//! per-request service builds its OWN `create_app` router and drives the resources tool
//! through temper-client's in-process door — and the LEDGER is checked for the emitter.
//! The `@mcp` surface can only have arrived through the trusted in-process extension:
//! no hop of the wiring may degrade it to `@web`.
//!
//! Bite (run manually against the pre-fix tree): removing the `InProcessSurface`
//! insertion from `temper-client`'s in-process send path makes the router fall back to
//! the untrusted header/web default and this test REDS with `@web` — the property
//! regressed exactly where the trap said it would.

mod common;

use jsonwebtoken::Algorithm;
use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::service::ServiceExt;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};

use temper_mcp::config::{McpConfig, OAuthStaticConfig};
use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

/// The emitter entity name the `resource_created` event carries — same ledger query
/// G1's door witness runs, now against an act that crossed the REAL MCP server.
async fn resource_created_emitter(pool: &sqlx::PgPool, resource_id: &str) -> String {
    sqlx::query_scalar(
        "SELECT ent.name \
         FROM kb_events ev \
         JOIN kb_event_types et ON et.id = ev.event_type_id \
         JOIN kb_entities ent ON ent.id = ev.emitter_entity_id \
         WHERE et.name = 'resource_created' \
           AND ev.payload->>'resource_id' = $1",
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("the create event carries its emitter")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_resources_act_through_the_real_mcp_server_attributes_to_mcp(pool: sqlx::PgPool) {
    // Provision through the API harness first: the approved profile + a writable
    // context, exactly the standing a deployed profile arrives with.
    let app = common::setup(pool.clone()).await;
    let ctx = app
        .client
        .contexts()
        .create("mcp-resources-attribution", None)
        .await
        .expect("context create failed");

    // The MCP router as its OWN deployment — the topology api/mcp.rs runs — sharing
    // only the database and the auth configuration with the API harness above.
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
        blob: None,
        blob_disabled_by_policy: false,
    };
    let api_state = AppState::new(pool, jwks_store, api_config);
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
    let mcp_addr = listener.local_addr().expect("mcp addr");
    tokio::spawn(async move {
        axum::serve(listener, temper_mcp::build_router(api_state, mcp_config))
            .await
            .expect("mcp server");
    });

    // A conformant MCP client over real HTTP: initialize handshake, then the tool call.
    // `auth_header` takes the RAW token and adds the `Bearer ` prefix itself; the
    // stateless-tolerant config default matches the deployed `stateful_mode(false)`.
    let transport = StreamableHttpClientTransport::with_client(
        reqwest13::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(format!("http://{mcp_addr}/mcp"))
            .auth_header(app.token.clone()),
    );
    let service = ClientInfo::default()
        .serve(transport)
        .await
        .expect("the initialize handshake must succeed over the deployed transport");
    let peer = service.peer().clone();

    // The resources act, over the wire: create with the context as home. The arguments
    // are the tool's WIRE shape — what an agent actually sends.
    let params = CallToolRequestParams::new("create_resource".to_owned()).with_arguments(
        [
            (
                "context_ref".to_owned(),
                serde_json::json!(ctx.id.to_string()),
            ),
            ("doc_type_name".to_owned(), serde_json::json!("session")),
            (
                "title".to_owned(),
                serde_json::json!("Attributed through the real MCP server"),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let result = peer
        .call_tool(params)
        .await
        .expect("create_resource over the deployed transport");
    let text = match result.content.first().map(|c| &c.raw) {
        Some(rmcp::model::RawContent::Text(t)) => t.text.clone(),
        other => panic!("tool returned no text content part: {other:?}"),
    };
    let created: serde_json::Value =
        serde_json::from_str(&text).expect("the create response parses");
    let resource_id = created["resource"]["id"]
        .as_str()
        .expect("the create response carries the resource id")
        .to_string();

    // The ledger is the judge: the emitter names the caller under the MCP surface.
    let emitter = resource_created_emitter(&app.pool, &resource_id).await;
    assert!(
        emitter.ends_with("@mcp"),
        "an MCP-originated resources act must attribute to the mcp emitter at the \
         ledger, got {emitter:?}"
    );
}
