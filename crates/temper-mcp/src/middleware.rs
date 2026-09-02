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
use temper_services::state::KeyLookupError;

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

    let vk = match state
        .api_state
        .jwks_store
        .get_decoding_key_for_token(&token)
        .await
    {
        Ok(k) => k,
        // A `kid` this instance does not publish is a bad TOKEN, so it takes the same 401 +
        // `WWW-Authenticate` path as any other bad token — which is what makes an MCP client
        // re-run its OAuth flow after the signing key rotates. Reporting it as 503 would tell the
        // client to come back later and never re-authenticate, and would let anyone make this
        // surface claim it is down by naming a `kid` at random.
        Err(KeyLookupError::UnknownKid(kid)) => {
            // Unverified-header value: attacker-chosen length. Bounded.
            tracing::debug!(kid = %temper_services::error::bounded(&kid), "token names an unpublished kid");
            return unauthorized(&state);
        }
        Err(e) => {
            tracing::error!("JWKS retrieval failed: {e}");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };

    // The MCP surface validates its own RFC 8707 resource audience — `mcp_audience`, the value
    // its PRM advertises and what conformant MCP clients request — and still accepts the API
    // audience: machine tokens (`client_credentials`) and sessions minted before `MCP_AUDIENCE`
    // was introduced carry it, and both surfaces are one instance, so a token naming either
    // audience names us. The audience split exists to satisfy MCP clients' client-side PRM check
    // (resource must equal the MCP server URL or its origin), not to separate trust domains.
    // When `MCP_AUDIENCE` is unset the two resolve to one value and this is the single-audience
    // check it always was — which is also why the set is deduped.
    let auth = &state.api_state.config.auth;
    let audiences: &[&str] = if auth.mcp_audience == auth.audience {
        &[&auth.audience]
    } else {
        &[&auth.mcp_audience, &auth.audience]
    };
    let validation = state
        .api_state
        .jwks_store
        .validation(&auth.issuer, audiences, vk.algorithm);

    match decode::<RawJwtClaims>(&token, &vk.key, &validation) {
        Ok(data) => {
            // The token verified, so this caller is one of ours: join its trace by link, never by
            // parenting (decision `019f95ff` rule 2). Here rather than beside `profile_id` in
            // `service.rs` because the trust event is the token verifying — and because
            // `Span::current()` is unambiguously the `mcp_request` root span in a middleware,
            // whereas inside a tool call it is whatever span the rmcp dispatch has entered.
            // MCP is the mention flow's last hop, so a link missing here loses the join the goal
            // exists to prove.
            temper_telemetry::link_trusted_caller(&tracing::Span::current(), request.headers());
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
