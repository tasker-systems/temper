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
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::trace::{DefaultOnFailure, TraceLayer};
use tracing::Span;

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
    let config = StreamableHttpServerConfig::default()
        .with_stateful_mode(false)
        .with_json_response(true);

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
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::extract::Request| {
                    tracing::info_span!(
                        "mcp_request",
                        method = %request.method(),
                        path = %request.uri().path(),
                        version = ?request.version(),
                        profile_id = tracing::field::Empty,
                    )
                })
                .on_response(
                    |response: &axum::response::Response, latency: Duration, _: &Span| {
                        tracing::info!(
                            status = response.status().as_u16(),
                            latency_ms = latency.as_millis() as u64,
                            "response",
                        );
                    },
                )
                .on_failure(DefaultOnFailure::new().level(tracing::Level::ERROR)),
        )
        .layer(CorsLayer::permissive())
}
