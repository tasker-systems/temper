//! Witness that the MCP surface's cross-origin policy comes from configuration, not from a
//! hardcoded constant.
//!
//! `CORS_ORIGINS` is read by both HTTP surfaces (`create_app` and `create_internal_app` share
//! `apply_transport_layers`, which builds its `CorsLayer` from `state.config.cors_origins`). The
//! MCP router assembled its own stack and ended in a literal `CorsLayer::permissive()`, so the
//! configured value reached the process, was parsed into `ApiConfig`, and was then dropped at the
//! one layer that acts — on the surface that takes the most automated traffic, with nothing
//! reporting the discrepancy.
//!
//! These two halves fail in opposite directions against the hardcoded-permissive state, which is
//! what makes them a witness for *derived from config* rather than for any particular value:
//!   - deny-all config (`cors_origins: []`) must yield **no** `access-control-allow-origin`;
//!     permissive answers `*`.
//!   - allowlist config must echo **that origin**; permissive answers `*` here too.
//!
//! No database and no port: `connect_lazy` builds an `AppState` whose pool is never queried (same
//! device as `dispatch_witness_test.rs`), and the probe targets `/mcp/health`, the one public
//! route on this router — so neither auth nor a tool body is reached.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sqlx::postgres::PgPoolOptions;
use temper_mcp::config::{McpConfig, OAuthStaticConfig};
use temper_services::{
    auth_config::{AuthConfig, AuthMode},
    config::ApiConfig,
    state::{AppState, JwksKeyStore},
};
use tower::ServiceExt;

const PROBE_ORIGIN: &str = "https://app.example.com";

/// An `AppState` carrying `cors_origins` and nothing else that matters here.
fn state_with_cors_origins(cors_origins: Vec<String>) -> AppState {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://__cors_witness_no_db__")
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
        cors_origins,
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
    };

    let jwks = JwksKeyStore::new("https://example.invalid/.well-known/jwks.json".to_string());
    AppState::new(pool, jwks, config)
}

fn mcp_config() -> McpConfig {
    McpConfig {
        mcp_base_url: "https://temper.invalid".to_string(),
        mcp_client_id: None,
        oauth: OAuthStaticConfig {
            redirect_uris: vec![],
            allow_localhost: false,
        },
    }
}

/// Send a cross-origin GET to the router's public health route and return the
/// `access-control-allow-origin` it answered with, if any.
async fn allow_origin_for(cors_origins: Vec<String>) -> Option<String> {
    let router = temper_mcp::build_router(state_with_cors_origins(cors_origins), mcp_config());

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/mcp/health")
                .header(header::ORIGIN, PROBE_ORIGIN)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router answers");

    // The probe is only meaningful if it actually reached the public route — a 404 or a 401 would
    // make an absent CORS header prove nothing about the policy.
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/mcp/health must be reachable without auth for this probe to mean anything"
    );

    response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .map(|v| v.to_str().expect("header is ASCII").to_string())
}

/// No configured origins means deny-all on the HTTP surfaces (`cors_layer`'s first branch says so
/// in as many words). MCP must agree rather than answering `*`.
#[tokio::test]
async fn no_configured_origins_denies_cross_origin_on_mcp() {
    let allow_origin = allow_origin_for(vec![]).await;

    assert_eq!(
        allow_origin, None,
        "an unconfigured CORS_ORIGINS must deny cross-origin on MCP as it does on the HTTP API; \
         answering {allow_origin:?} means the MCP router is not reading the configured value"
    );
}

/// A configured allowlist must be echoed back as itself. `*` here is the same defect as the case
/// above wearing a different answer: it proves the layer ignored the configuration.
#[tokio::test]
async fn configured_origin_is_echoed_rather_than_wildcarded_on_mcp() {
    let allow_origin = allow_origin_for(vec![PROBE_ORIGIN.to_string()]).await;

    assert_eq!(
        allow_origin.as_deref(),
        Some(PROBE_ORIGIN),
        "a configured allowlist must be honored on MCP; `*` or absence means the configured value \
         never reached the layer that acts"
    );
}
