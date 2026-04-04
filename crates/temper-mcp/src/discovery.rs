//! OAuth well-known discovery endpoints.
//!
//! These endpoints tell MCP clients how to authenticate. Auth0 is the
//! authorization server; we just advertise its endpoints.

use axum::{extract::State, response::IntoResponse, Json};
use serde_json::json;
use std::sync::Arc;

use crate::router::McpAppState;

/// RFC 9728 — Protected Resource Metadata.
/// Tells MCP clients which authorization server issues tokens for this resource.
pub async fn oauth_protected_resource(
    State(state): State<Arc<McpAppState>>,
) -> impl IntoResponse {
    let base = &state.mcp_config.mcp_base_url;
    let auth0 = state.mcp_config.auth0_domain.trim_end_matches('/');

    Json(json!({
        "resource": format!("{base}/"),
        "authorization_servers": [format!("{auth0}/")],
        "bearer_methods_supported": ["header"],
        "scopes_supported": ["openid", "profile", "email"],
    }))
}

/// RFC 8414 — Authorization Server Metadata.
/// Returns Auth0's OAuth endpoints so MCP clients can perform the PKCE flow.
pub async fn oauth_authorization_server(
    State(state): State<Arc<McpAppState>>,
) -> impl IntoResponse {
    let auth0 = state.mcp_config.auth0_domain.trim_end_matches('/');
    let audience = &state.mcp_config.mcp_audience;

    Json(json!({
        "issuer": format!("{auth0}/"),
        "authorization_endpoint": format!("{auth0}/authorize"),
        "token_endpoint": format!("{auth0}/oauth/token"),
        "scopes_supported": ["openid", "profile", "email"],
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "resource": audience,
    }))
}
