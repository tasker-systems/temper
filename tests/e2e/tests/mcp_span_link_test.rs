#![cfg(feature = "test-db")]
//! The span-link gate on the **MCP** surface — parity with `temper-api`'s
//! `telemetry_link_test`, and not a formality.
//!
//! MCP is the mention flow's last hop, so a link that goes missing here loses exactly the join the
//! whole goal exists to prove. It is also the surface where the claim is least self-evident: the
//! link is recorded on `Span::current()` inside `require_mcp_auth`, and whether that resolves to the
//! `mcp_request` root span is a property of `build_router`'s layer *order* — `root_span` is applied
//! to the merged router and `require_mcp_auth` only to `mcp_routes`, so the root runs outermost.
//! That is a reading of tower semantics, and the last time this task reasoned about layer ordering
//! instead of measuring it (the flush that saw zero spans), the reasoning was wrong.
//!
//! Its own file because the tracer provider and the subscriber are process-global and install once.

mod common;

use opentelemetry_sdk::trace::InMemorySpanExporter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

const SPEC_TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
const SPEC_TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_links_an_authenticated_callers_trace_and_no_one_elses(pool: sqlx::PgPool) {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "a provider was already installed — this test owns the process"
    );
    let layer = temper_telemetry::export::test_export_layer()
        .expect("the layer must exist once a provider is installed");
    tracing_subscriber::registry().with(layer).init();

    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("decoding key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, jsonwebtoken::Algorithm::RS256);
    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: common::TEST_AUDIENCE.to_string(),
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
    };
    let state = AppState::new(pool, jwks_store, api_config);
    let mcp_config = temper_mcp::McpConfig {
        mcp_base_url: "http://localhost".to_string(),
        mcp_client_id: None,
        oauth: temper_mcp::config::OAuthStaticConfig {
            redirect_uris: vec![],
            allow_localhost: true,
        },
    };
    let app = temper_mcp::build_router(state, mcp_config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mcp test server");
    });

    let client = reqwest::Client::new();
    let token = common::generate_test_jwt("test|mcp-link", "mcp-link@example.com");

    // Authenticated. The MCP protocol exchange beyond the middleware is irrelevant here — what is
    // being observed is the root span the transport layer exported, and `require_mcp_auth` has run
    // by the time anything downstream can answer at all.
    let authenticated = client
        .post(format!("http://{addr}/mcp"))
        .header("Authorization", format!("Bearer {token}"))
        .header("traceparent", SPEC_TRACEPARENT)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "span-link-test", "version": "0" }
            }
        }))
        .send()
        .await
        .expect("authenticated mcp request");
    assert_ne!(
        authenticated.status().as_u16(),
        401,
        "the bearer did not authenticate, so this half of the test would prove nothing"
    );

    // The same header, no bearer.
    let anonymous = client
        .post(format!("http://{addr}/mcp"))
        .header("traceparent", SPEC_TRACEPARENT)
        .header("content-type", "application/json")
        .send()
        .await
        .expect("anonymous mcp request");
    assert_eq!(anonymous.status().as_u16(), 401);

    let spans = exporter
        .get_finished_spans()
        .expect("in-memory exporter readable");
    // The two request root spans, located by server kind rather than by name: the exported name is
    // now the per-route `{method} {route}` (both requests here are `POST /mcp`), while `otel.kind =
    // server` still marks the request boundary. temper-mcp reaches temper-services in-process, so the
    // only server-kind spans are these two roots — any tool-dispatch or act spans are internal.
    let mcp_spans: Vec<_> = spans
        .iter()
        .filter(|span| span.span_kind == opentelemetry::trace::SpanKind::Server)
        .collect();
    assert_eq!(
        mcp_spans.len(),
        2,
        "expected one `mcp_request` root span per request; got {}: {mcp_spans:#?}",
        mcp_spans.len()
    );

    let linked: Vec<_> = mcp_spans
        .iter()
        .filter(|span| !span.links.is_empty())
        .collect();
    assert_eq!(
        linked.len(),
        1,
        "exactly one of the two requests authenticated, so exactly one span should carry a link. \
         Two means the link is attached before `require_mcp_auth` and any caller can graft onto \
         our traces; zero means `Span::current()` in that middleware is not the root span, and the \
         link is being recorded onto nothing: {mcp_spans:#?}"
    );
    let link = linked[0].links.iter().next().expect("the link");
    assert_eq!(
        link.span_context.trace_id().to_string(),
        SPEC_TRACE_ID,
        "the link does not point at the caller's trace: {:#?}",
        linked[0]
    );
    assert_ne!(
        linked[0].span_context.trace_id().to_string(),
        SPEC_TRACE_ID,
        "the MCP root span was rooted inside the caller's trace — decision 019f95ff rule 1: {:#?}",
        linked[0]
    );

    // Non-vacuity: both requests recorded the inbound context as inert fields (rule 4), so the
    // one-and-only-one count above is about *authentication* rather than about the header being
    // dropped on one path.
    for span in &mcp_spans {
        assert!(
            span.attributes
                .iter()
                .any(|kv| kv.key.as_str() == "trace_id" && kv.value.as_str() == SPEC_TRACE_ID),
            "an `mcp_request` span did not record `trace_id`, so the link count proves nothing \
             about auth: {span:#?}"
        );
    }
}
