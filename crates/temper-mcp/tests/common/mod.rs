// Each integration test file compiles this module into its own binary, so a fixture used by one
// suite is dead code in the others. That is inherent to `tests/common`, not a sign of an unused
// helper.
#![allow(dead_code)]

//! Fixtures for the router-level integration tests.
//!
//! Both suites here assemble a real `build_router` and drive requests through it. Neither reaches a
//! tool body, so the pool is built with `connect_lazy` and never queried — same device as
//! `dispatch_witness_test.rs`, which needs an `AppState` for a service rather than a router.

use jsonwebtoken::{Algorithm, DecodingKey};
use sqlx::postgres::PgPoolOptions;
use temper_mcp::config::{McpConfig, OAuthStaticConfig};
use temper_services::{
    auth_config::{AuthConfig, AuthMode},
    config::ApiConfig,
    state::{AppState, JwksKeyStore},
};

/// An `AppState` whose key store is pre-loaded, so the auth gate actually *validates* a token
/// instead of failing to fetch a key.
///
/// This distinction has teeth. With the unreachable JWKS URL that [`state_with_cors_origins`]
/// uses, a request carrying a malformed bearer is refused with `503` — the store cannot fetch a
/// key, so the gate fails closed before it can judge the token. That is the correct direction to
/// fail, but it means a `401` assertion against that fixture would be testing the network, not the
/// gate. A static key makes the refusal a real validation verdict.
pub fn state_with_static_jwt_key() -> AppState {
    let mut state = state_with_cors_origins(vec![]);
    let jwks =
        JwksKeyStore::with_static_key(DecodingKey::from_secret(b"witness"), Algorithm::HS256);
    state.jwks_store = std::sync::Arc::new(jwks);
    state
}

/// An `AppState` carrying `cors_origins` and nothing else that matters to a router test.
pub fn state_with_cors_origins(cors_origins: Vec<String>) -> AppState {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://__router_witness_no_db__")
        .expect("lazy pool constructs without a server");

    let config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "unused".to_string(),
            jwks_url: "unused".to_string(),
            audience: "unused".to_string(),
            mcp_audience: "unused".to_string(),
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

/// An `AppState` whose auth identity names DISTINCT API and MCP audiences, with the same static
/// HS256 key as [`state_with_static_jwt_key`] — the dedicated-MCP-resource instance shape. The
/// audience-set tests pin the gate's contract against it: a token for either audience passes,
/// a token for neither does not.
pub fn state_with_distinct_audiences() -> AppState {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://__router_witness_no_db__")
        .expect("lazy pool constructs without a server");

    let config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "https://as.test".to_string(),
            jwks_url: "unused".to_string(),
            audience: "https://inst.test/api".to_string(),
            mcp_audience: "https://inst.test/mcp".to_string(),
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

    let jwks =
        JwksKeyStore::with_static_key(DecodingKey::from_secret(b"witness"), Algorithm::HS256);
    AppState::new(pool, jwks, config)
}

/// An `McpConfig` with **no** `mcp_client_id`, which is what makes `/oauth/register` answer
/// `503 SERVICE_UNAVAILABLE` from inside the handler rather than failing earlier.
pub fn mcp_config() -> McpConfig {
    McpConfig {
        mcp_base_url: "https://temper.invalid".to_string(),
        mcp_client_id: None,
        oauth: OAuthStaticConfig {
            redirect_uris: vec![],
            allow_localhost: false,
        },
    }
}
