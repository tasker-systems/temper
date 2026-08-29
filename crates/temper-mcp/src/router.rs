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
use tower_http::limit::RequestBodyLimitLayer;

use temper_services::state::AppState;

use crate::config::McpConfig;
use crate::discovery;
use crate::middleware::require_mcp_auth;
use crate::service::TemperMcpService;

/// The largest request body this door will read.
///
/// `[added — 2026-08-28, found in review]` **This door had no body limit of any kind**, and not by
/// omission: `/mcp` is mounted with `nest_service`, whose target is a raw tower service, so no axum
/// extractor runs and `DefaultBodyLimit` is not merely unset but *inapplicable*. rmcp's `expect_json`
/// reads the body with a bare `.collect()`, and `apply_base_layers` puts request decompression
/// ABOVE this router — so a compressed POST expanded into an unbounded buffer on a ~1.7 vCPU
/// function. `RequestBodyLimitLayer` is the instrument that works on a raw service, and sitting
/// inside decompression it measures decompressed bytes, the same property `/api/query`'s limit has.
///
/// **Why this is not `QUERY_MAX_BODY_BYTES`.** `/api/query` reads one composition and 4 MB is
/// generous for it. This door carries the whole tool surface, including `ingest`'s inline
/// `content: String` and `data_artifacts`' `content: serde_json::Value`, so a 4 MB ceiling could
/// refuse legitimate work — and lowering a limit later is the breaking direction. 25 MB matches
/// `GITHUB_MAX_WEBHOOK_BYTES`, this repo's existing generous transport bound, which puts the number
/// on an in-repo precedent rather than on a guess.
///
/// **What it does NOT bound**, stated so it is not mistaken for more than it is: every declaration
/// cap on a composition is enforced identically on this door, because `run_query` calls the same
/// `query_read::prepare`. What a transport limit adds is the backstop for the cost the declaration
/// caps cannot see — `QUERY_MAX_BODY_BYTES`' own doc names it, a single `Contains` value counting
/// as one probe however large. At 25 MB that shape is bounded far more loosely here than at 4 MB on
/// the HTTP door. Closing it properly is a declaration-cap question, not a transport one.
const MCP_MAX_BODY_BYTES: usize = 25 * 1024 * 1024;

/// Shared state for discovery handlers and the MCP middleware.
#[derive(Clone, Debug)]
pub struct McpAppState {
    pub api_state: AppState,
    pub mcp_config: McpConfig,
}

pub fn build_router(api_state: AppState, mcp_config: McpConfig) -> Router {
    // Taken before `api_state` is moved into the service factory below. `AppState::config` is an
    // `Arc`, so this is a refcount bump rather than a copy of the configuration.
    let cors_config = api_state.config.clone();

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

    // The limit is the OUTERMOST layer here, above `require_mcp_auth`, so an oversized body is
    // refused without the database round trip that authentication performs. It is still INSIDE
    // `apply_base_layers`' decompression, which is applied to the merged router below — so it
    // measures decompressed bytes, the property `/api/query`'s limit has and the one that matters.
    let mcp_routes = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(middleware::from_fn_with_state(
            shared.clone(),
            require_mcp_auth,
        ))
        .layer(RequestBodyLimitLayer::new(MCP_MAX_BODY_BYTES));

    // ── Health (public) ────────────────────────────────────────────────
    let health = Router::new().route("/mcp/health", get(|| async { "ok" }));

    // The layers below the root span, from the same place temper-api takes them: the structured
    // 404 and request decompression. Applied to the merged router so the fallback outranks the one
    // `mcp_routes` inherits — without it an unmatched path was answered by the auth middleware
    // wrapping that router's fallback, so a typo'd URL came back `401`, not a 404 of any shape.
    temper_services::transport::apply_base_layers(
        Router::new()
            .merge(discovery_routes)
            .merge(registration_routes)
            .merge(health)
            .merge(mcp_routes),
    )
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
    // The same cross-origin policy the HTTP surfaces apply, from the same configured value.
    // This was `CorsLayer::permissive()` — a literal, so `CORS_ORIGINS` was parsed into
    // `ApiConfig`, carried here inside `AppState`, and then dropped at the one layer that
    // acts. Tightening the allowlist changed nothing on the agent-facing door and nothing
    // reported that. `temper_services::cors` now owns the policy so there is one place it
    // can be read from and no second stack to forget.
    .layer(temper_services::cors::cors_layer(&cors_config))
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
