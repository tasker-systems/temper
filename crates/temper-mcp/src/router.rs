//! Router assembly — combines OAuth discovery, health, registration, and the MCP endpoint.

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

use temper_services::state::AppState;

use crate::config::McpConfig;
use crate::discovery;
use crate::middleware::require_mcp_auth;
use crate::service::TemperMcpService;

/// Shared state for discovery handlers and the MCP middleware.
#[derive(Clone, Debug)]
pub struct McpAppState {
    pub api_state: AppState,
    pub mcp_config: McpConfig,
}

pub fn build_router(api_state: AppState, mcp_config: McpConfig) -> Router {
    let shared = Arc::new(McpAppState {
        api_state: api_state.clone(),
        mcp_config,
    });

    // ── Public OAuth discovery endpoints ───────────────────────────────
    let discovery_routes = Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(discovery::oauth_protected_resource),
        )
        .with_state(shared.clone());

    // ── Public OAuth registration (thin DCR proxy) ─────────────────────
    // Returns the pre-registered Auth0 client_id to MCP clients like
    // Claude Desktop so they can complete OAuth without manual entry.
    let registration_routes = Router::new()
        .route("/oauth/register", post(discovery::register_client))
        .with_state(shared.clone());

    // ── Protected MCP endpoint ─────────────────────────────────────────
    // StreamableHttpService handles POST /mcp, GET /mcp (SSE), DELETE /mcp.
    // Using stateless mode (json_response + !stateful_mode) for Vercel
    // serverless compatibility — each invocation is independent.
    //
    // `disable_allowed_hosts`: rmcp 1.4+ added DNS-rebinding protection that rejects any `Host`
    // header not in a loopback-only allowlist (`localhost`, `127.0.0.1`, `::1`). On Vercel the
    // `Host` header is the deployment domain (production `temperkb.io`, dynamic preview URLs),
    // so the default allowlist would 400 every production request. Temper's auth middleware
    // (`require_mcp_auth`) is the real gate here, and a static host allowlist cannot track
    // Vercel's per-deployment preview domains. The rebinding check is a local-server guard and
    // is not the right gate for a serverless deployment behind Vercel's edge + temper's own auth.
    let config = StreamableHttpServerConfig::default()
        .with_stateful_mode(false)
        .with_json_response(true)
        .disable_allowed_hosts();

    let mcp_service = StreamableHttpService::new(
        move || Ok(TemperMcpService::new(api_state.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let mcp_routes =
        Router::new()
            .nest_service("/mcp", mcp_service)
            .layer(middleware::from_fn_with_state(
                shared.clone(),
                require_mcp_auth,
            ));

    // ── Health (public) ────────────────────────────────────────────────
    let health = Router::new().route("/mcp/health", get(|| async { "ok" }));

    Router::new()
        .merge(discovery_routes)
        .merge(registration_routes)
        .merge(health)
        .merge(mcp_routes)
        // HTTP root span, mirroring temper-api's `apply_transport_layers`. Until this landed, MCP
        // requests had NO root span at all — every MCP log line was parentless, on the surface that
        // carries the most automated traffic. The span name is `mcp_request`, deliberately NOT the
        // `http_request` that temper-api's root span and temper-client's request span both already
        // use: three different things under one name is unreadable once they are exported together.
        //
        // `profile_id` is declared Empty and recorded in `service.rs`, not in `require_mcp_auth` —
        // that middleware only validates the JWT, and a validated token is not yet a profile. Same
        // deferred-field pattern temper-api uses in its auth middleware, one seam further in.
        .layer(axum::middleware::from_fn(root_span))
        .layer(CorsLayer::permissive())
}

/// The `mcp_request` root span, and the end of its life.
///
/// Replaced `tower_http`'s `TraceLayer` when the exporter landed — it clones its span into the
/// response body, which outlives every middleware, so a flush could never see the request's own
/// span. `temper_telemetry::request_span` carries the measurement. Name, fields, and the `response`
/// event are unchanged.
async fn root_span(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // One expansion of the same macro temper-api uses, so the field set cannot drift between the
    // surfaces. Parity matters more here than anywhere: the mention flow's last hop lands on MCP,
    // so a trace that stops at the API boundary stops one hop short of the work it was following.
    temper_telemetry::traced_request(request, next, |request| {
        temper_telemetry::root_span!("mcp_request", request)
    })
    .await
}
