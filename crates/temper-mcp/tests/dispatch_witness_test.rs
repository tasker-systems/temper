//! Witness that `#[tool_handler]` actually wires `ServerHandler::call_tool` into the router.
//!
//! The existing tests in `service.rs` assert what the router *contains* — they call
//! `TemperMcpService::tool_router()` (a pure associated function) and inspect its advertised
//! tools. None of them drive `ServerHandler::call_tool`, the generated entry point a real MCP
//! client reaches. So a bump of `rmcp` that silently changed the `#[tool_handler]` default —
//! rebuilding a fresh router per call instead of reading the stored field, orphaning it — was
//! only noticed by a `-D dead-code` lint. Had the bump broken routing outright rather than
//! orphaning a field, every router-contents test would still have passed. That is a gate that
//! cannot fail for the thing it appears to cover.
//!
//! This test drives the generated `call_tool` through a real `RequestContext` built off a
//! served `RunningService`'s `Peer` (the only public way to obtain one — `Peer::new` is
//! `pub(crate)` in rmcp). It uses `serve_directly`, which skips the client-handshake
//! initialization that `serve().await` waits for and returns a `RunningService` synchronously.
//! The witness does NOT exercise any tool body: both assertions fire at the dispatch layer,
//! before auth or the database are reached.
//!
//! The two halves together prove the full dispatch path is wired:
//!   - An **unknown tool** is refused by the router with `INVALID_PARAMS` "tool not found" —
//!     proving `call_tool` reached the router at all. Without `#[tool_handler]`, `call_tool`
//!     falls back to the trait default which returns `METHOD_NOT_FOUND`.
//!   - A **known tool** is dispatched to its wrapper, which fails extracting
//!     `Extension<http::request::Parts>` (absent in this bare context) with `INVALID_PARAMS`
//!     "missing extension …" — proving the router found the tool and handed off to its wrapper.
//!     Without `#[tool_handler]`, this too returns `METHOD_NOT_FOUND`.
//!
//! Both halves fail (become `METHOD_NOT_FOUND`) if `#[tool_handler]` is removed from the
//! `impl ServerHandler` block — that is the regression boundary this witness guards.

use rmcp::{
    model::{CallToolRequestParams, ErrorCode, RequestId},
    service::{serve_directly, RequestContext},
    ServerHandler,
};
use sqlx::postgres::PgPoolOptions;
use temper_mcp::service::TemperMcpService;
use temper_services::{
    auth_config::{AuthConfig, AuthMode},
    config::ApiConfig,
    state::{AppState, JwksKeyStore},
};

/// Build a `TemperMcpService` backed by a lazy (never-connected) pool.
///
/// The dispatch witness never reaches a tool body, so the pool is never used — it only has to
/// exist for `AppState::new`. `connect_lazy` produces a pool that will not attempt a connection
/// until a query is issued, which this test never does.
fn service_for_dispatch_witness() -> TemperMcpService {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://__witness_no_db__")
        .expect("lazy pool constructs without a server");

    let config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "unused".to_string(),
            jwks_url: "unused".to_string(),
            audience: "unused".to_string(),
            mode: AuthMode::ExternalIdp,
        },
        auth_provider_name: "unused".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
    };

    let jwks = JwksKeyStore::new("https://example.invalid/.well-known/jwks.json".to_string());
    let state = AppState::new(pool, jwks, config);
    TemperMcpService::new(state)
}

/// Dispatch a tool call through the generated `ServerHandler::call_tool` against a bare
/// `RequestContext` (no HTTP parts, no auth, no DB) and return the resulting error.
///
/// Both witness assertions expect an *error* — dispatch reaches the router but fails before any
/// tool body runs — so this helper wraps the "expect an error" plumbing. The service is served
/// directly (skipping the init handshake) on a throwaway duplex transport; the served `Peer` is
/// `Arc`-backed so it outlives the `RunningService`. The same service instance is then driven
/// through `call_tool` (it is `Clone`).
async fn dispatch_fails(service: TemperMcpService, name: &'static str) -> rmcp::ErrorData {
    let (server_io, _client_io) = tokio::io::duplex(4096);
    let running = serve_directly(service.clone(), server_io, None);
    let ctx = RequestContext::new(RequestId::Number(1), running.peer().clone());
    ServerHandler::call_tool(&service, CallToolRequestParams::new(name), ctx)
        .await
        .expect_err("dispatch must reach the router and fail there, not succeed")
}

/// The error from dispatching an **unknown** tool through `ServerHandler::call_tool` is
/// `INVALID_PARAMS` "tool not found" — the router was reached and refused the name.
///
/// Without `#[tool_handler]` this becomes `METHOD_NOT_FOUND` (the trait default), which is the
/// regression this witness exists to catch.
#[tokio::test]
async fn an_unknown_tool_is_refused_by_the_router_not_by_the_default_handler() {
    let service = service_for_dispatch_witness();
    let err = dispatch_fails(service, "definitely_not_a_real_tool").await;

    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "unknown-tool dispatch returned {:?} ({:?}); expected INVALID_PARAMS (\"tool not \
         found\"). METHOD_NOT_FOUND would mean `#[tool_handler]` is not wiring call_tool into the \
         router — the regression this witness guards.",
        err.code,
        err.message,
    );
    assert!(
        err.message.contains("tool not found"),
        "unknown-tool refusal message changed: {:?}. The witness keys on this string; if rmcp \
         reworded it, update the assertion to the new message — do not weaken it to a code-only \
         check, because the message is what distinguishes \"router reached\" from other \
         INVALID_PARAMS failures.",
        err.message,
    );
}

/// The error from dispatching a **known** tool through `ServerHandler::call_tool` is
/// `INVALID_PARAMS` "missing extension http::request::Parts" — the router found the tool and
/// handed off to its wrapper, which failed extracting the HTTP parts this bare context does not
/// carry.
///
/// Without `#[tool_handler]` this becomes `METHOD_NOT_FOUND` (the trait default).
///
/// This is the half that proves the router routes *to a real tool*, not merely that it was
/// reached. The unknown-tool test alone could pass against a router that refused everything;
/// this one confirms a known name is dispatched.
#[tokio::test]
async fn a_known_tool_is_dispatched_to_its_wrapper_not_the_default_handler() {
    let service = service_for_dispatch_witness();
    let err = dispatch_fails(service, "search").await;

    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "known-tool dispatch returned {:?} ({:?}); expected INVALID_PARAMS (\"missing \
         extension …\"). METHOD_NOT_FOUND would mean `#[tool_handler]` is not wiring call_tool \
         into the router — the regression this witness guards.",
        err.code,
        err.message,
    );
    assert!(
        err.message.contains("missing extension"),
        "known-tool dispatch message changed: {:?}. The wrapper extracts \
         `Extension<http::request::Parts>` from the request context and fails when it is absent \
         (this bare context carries none). If the message moved, update the assertion to the new \
         wording — do not weaken it to a code-only check, because the message is what distinguishes \
         \"wrapper reached\" from other INVALID_PARAMS failures.",
        err.message,
    );
}
