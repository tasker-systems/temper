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

use temper_api::state::AppState;

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
        .route(
            "/.well-known/oauth-authorization-server",
            get(discovery::oauth_authorization_server),
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
        .layer(CorsLayer::permissive())
}
