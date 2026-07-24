//! The span-field convention gate.
//!
//! The convention has two clauses, and they are deliberately different in strength:
//!
//! 1. **Every request produces a root span carrying the request-level fields** (`method`, `path`,
//!    plus `profile_id` once authenticated). Unconditional.
//! 2. **When an act exists, its ids appear on a span in that request's tree** — and on a span of
//!    their own, not merely merged into the root. Conditional *by design*: temper's C/U/D commands
//!    are Acts with correlation ids; a read is just a request, with no command-action mechanics
//!    behind it. Asserting act ids on every request would encode a fiction.
//!
//! Clause 2's "on a span of their own" is the part that has teeth. `correlation_id` and
//! `invocation_id` arrive in the request *body*, so the `TraceLayer` root span cannot carry them —
//! and until there were child spans, recording them onto the current span silently landed them on
//! the root anyway. That worked, and would have kept working right up until the first nested span
//! made it wrong. The gate pins the structure, not just the presence of a value.

#![cfg(feature = "test-db")]

mod common;

use temper_core::types::authorship::ActInput;
use temper_core::types::ids::CorrelationId;
use temper_core::types::ingest::IngestPayload;
use temper_services::backend::ACT_SPAN_FIELDS;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use common::tracing_layer::TestTracingLayer;

/// Verify that a request to a protected endpoint produces tracing spans
/// with the expected structured fields (method, path, status, profile_id).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn request_produces_structured_spans(pool: sqlx::PgPool) {
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let app = common::setup(pool).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let events = captured.lock().unwrap();

    let has_request_span = events.iter().any(|e| {
        let sf = &e.span_fields;
        sf.contains_key("method") && sf.contains_key("path")
    });

    assert!(
        has_request_span,
        "expected a tracing event with method and path span fields, got: {events:#?}"
    );
}

/// Verify that an unauthenticated request produces a warn-level event.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unauthenticated_request_logs_warning(pool: sqlx::PgPool) {
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let app = common::setup(pool).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 401);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let events = captured.lock().unwrap();

    let has_auth_warning = events.iter().any(|e| e.level <= tracing::Level::WARN);

    assert!(
        has_auth_warning,
        "expected a WARN-level event for 401 response, got: {events:#?}"
    );
}

fn empty_payload(title: &str, ctx: &str, act: ActInput) -> IngestPayload {
    IngestPayload {
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://span-convention/{title}"),
        context_ref: format!("@me/{ctx}"),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        act,
        sources: Vec::new(),
    }
}

/// Convention clause 2: an act's ids land on an act span of their own, inside the request's tree.
///
/// The negative half is the point. Asserting only "some span carries `correlation_id`" would pass
/// just as happily if the id had fallen through onto the HTTP root span — the pre-act-span
/// behaviour, and the exact regression this gate exists to catch. So the carrying span is also
/// required NOT to be the root, identified by its `path` field.
///
/// This is also what proves `#[instrument]` survives `#[async_trait]`: if the attribute instrumented
/// only the boxed-future construction rather than its execution, the act span would close before
/// `act_context` ran and the ids would land on the root instead. That failure is invisible to a
/// presence-only assertion and fatal to the span tree.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn act_ids_land_on_an_act_span_not_the_root(pool: sqlx::PgPool) {
    let (layer, _events, spans) = TestTracingLayer::with_spans();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let app = common::setup(pool).await;
    let correlation = CorrelationId::new();

    app.client
        .ingest()
        .create(&empty_payload(
            "Span Convention Doc",
            "default",
            ActInput {
                correlation_id: Some(correlation),
                ..Default::default()
            },
        ))
        .await
        .expect("authored create should succeed");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let spans = spans.lock().unwrap();
    let expected = correlation.uuid().to_string();

    // Clause 1 — the request has a root span with the request-level fields.
    assert!(
        spans
            .iter()
            .any(|s| s.fields.contains_key("method") && s.fields.contains_key("path")),
        "expected an HTTP root span carrying method + path, got: {spans:#?}"
    );

    // Clause 2 — the act's correlation is on a span, and that span is not the root.
    let carriers: Vec<_> = spans
        .iter()
        .filter(|s| s.fields.get("correlation_id") == Some(&expected))
        .collect();
    assert!(
        !carriers.is_empty(),
        "no span carried correlation_id={expected}; got: {spans:#?}"
    );
    assert!(
        carriers.iter().any(|s| !s.fields.contains_key("path")),
        "correlation_id={expected} appeared ONLY on the HTTP root span — the act span either never \
         opened or closed before the ids were recorded. Carriers: {carriers:#?}"
    );
}

/// The act-span field set has one definition, and it is the producer's.
///
/// `ACT_SPAN_FIELDS` is exported from `temper-services` precisely so this gate cannot drift from the
/// `#[instrument]` attributes it describes: a copy of the list here would let the two disagree
/// silently, which is how a convention becomes documentation-only.
#[test]
fn act_span_field_set_is_declared_once() {
    assert_eq!(ACT_SPAN_FIELDS, ["correlation_id", "invocation_id"]);
}

/// Convention clause 1, on the MCP surface.
///
/// temper-mcp had no `TraceLayer` at all until this task: `rg -n "TraceLayer"` returned two lines,
/// both in temper-api. Every MCP log line was parentless — on the surface that carries the most
/// automated traffic, since it is where agents work.
///
/// The existing MCP e2e coverage builds `TemperMcpService` directly and never exercises the router,
/// so nothing would have noticed the layer being dropped. This drives real HTTP through
/// `build_router` instead. `/mcp/health` is public, which keeps the test about the transport layer
/// rather than about auth.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_requests_produce_a_root_span(pool: sqlx::PgPool) {
    use temper_services::auth_config::{AuthConfig, AuthMode};
    use temper_services::config::ApiConfig;
    use temper_services::state::{AppState, JwksKeyStore};

    let (layer, _events, spans) = TestTracingLayer::with_spans();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

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

    let resp = reqwest::get(format!("http://{addr}/mcp/health"))
        .await
        .expect("health request");
    assert_eq!(resp.status().as_u16(), 200);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let spans = spans.lock().unwrap();
    assert!(
        spans.iter().any(|s| s.name == "mcp_request"
            && s.fields.contains_key("method")
            && s.fields.contains_key("path")),
        "expected an `mcp_request` root span carrying method + path, got: {spans:#?}"
    );
}
