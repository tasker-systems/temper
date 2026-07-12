//! JWT validation middleware for the MCP endpoint.
//!
//! Re-uses temper-api's `JwksKeyStore` for token validation. Simpler than
//! the full `require_auth` middleware — we validate the JWT and inject the
//! decoded [`RawJwtClaims`] plus the raw [`BearerToken`], which the service
//! hands to `temper_services::auth::authenticate_token` to classify, resolve
//! and gate the principal.

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::decode;
use std::sync::Arc;

use temper_services::auth::RawJwtClaims;

use crate::router::McpAppState;

/// The raw, already-verified bearer token of the current request.
///
/// A newtype rather than a bare `String` so it cannot be confused with any other
/// string in the extensions map. It travels beside [`RawJwtClaims`] because the
/// shared auth seam's human email ladder may need to present it to the IdP's
/// `/userinfo` endpoint — the one rung that needs the token itself, not its claims.
#[derive(Debug, Clone)]
pub struct BearerToken(pub String);

/// Validate the Auth0 Bearer JWT on every MCP request.
///
/// On success, injects [`RawJwtClaims`] and [`BearerToken`] into request extensions.
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

    let vk = match state.api_state.jwks_store.get_decoding_key().await {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("JWKS retrieval failed: {e}");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };

    // Both surfaces read the ONE audience off the shared AuthConfig. temper-mcp used to carry its
    // own `mcp_audience`, parsed separately — two parsers for one concept, which is how the two
    // surfaces came to answer an empty value in opposite ways.
    let issuer = &state.api_state.config.auth.issuer;
    let audience = state.api_state.config.auth.audience.as_str();
    let validation = state
        .api_state
        .jwks_store
        .validation(issuer, audience, vk.algorithm);

    match decode::<RawJwtClaims>(&token, &vk.key, &validation) {
        Ok(data) => {
            request.extensions_mut().insert(data.claims);
            request.extensions_mut().insert(BearerToken(token));
            next.run(request).await
        }
        Err(e) => {
            // Deliberately does NOT log the expected issuer/audience. Anyone can trigger this line
            // by sending a garbage bearer, and these are precisely the two config values the boot
            // gate's errors go out of their way never to print. `error` names which check failed,
            // which is what an operator debugging a 401 actually needs.
            tracing::warn!(error = %e, "MCP JWT validation failed");
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
