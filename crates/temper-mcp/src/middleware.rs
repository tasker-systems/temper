//! JWT validation middleware for the MCP endpoint.
//!
//! Re-uses temper-api's `JwksKeyStore` for token validation. Simpler than
//! the full `require_auth` middleware — we validate the JWT and extract
//! `sub` for tool-level scoping, without resolving a temper Profile.

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::decode;
use serde::Deserialize;
use std::sync::Arc;

use crate::router::McpAppState;

/// Claims extracted from every validated MCP access token.
#[derive(Debug, Clone, Deserialize)]
pub struct McpClaims {
    pub sub: String,
    pub exp: i64,
}

/// Validate the Auth0 Bearer JWT on every MCP request.
///
/// On success, injects `McpClaims` into request extensions.
/// On failure, returns 401 with a `WWW-Authenticate` header that triggers
/// the MCP client's OAuth flow (per MCP 2025-03-26 auth spec).
pub async fn require_mcp_auth(
    State(state): State<Arc<McpAppState>>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let token = match extract_bearer(&request) {
        Some(t) => t,
        None => return unauthorized(&state),
    };

    let decoding_key = match state.api_state.jwks_store.get_decoding_key().await {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("JWKS retrieval failed: {e}");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };

    let issuer = &state.api_state.config.auth_issuer;
    let audience = state.mcp_config.mcp_audience.as_str();
    let validation = state
        .api_state
        .jwks_store
        .validation(issuer, Some(audience));

    match decode::<McpClaims>(&token, &decoding_key, &validation) {
        Ok(data) => {
            request.extensions_mut().insert(data.claims);
            next.run(request).await
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                issuer = %issuer,
                audience = %audience,
                "MCP JWT validation failed"
            );
            unauthorized(&state)
        }
    }
}

fn unauthorized(state: &McpAppState) -> Response {
    let base = &state.mcp_config.mcp_base_url;
    let www_auth = format!(
        r#"Bearer realm="temper", resource_metadata="{base}/.well-known/oauth-protected-resource""#
    );
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, www_auth)],
        "Authentication required",
    )
        .into_response()
}

fn extract_bearer(request: &Request<Body>) -> Option<String> {
    let h = request.headers().get(header::AUTHORIZATION)?;
    let v = h.to_str().ok()?;
    v.strip_prefix("Bearer ").map(|s| s.to_string())
}
