//! OAuth well-known discovery endpoints.
//!
//! These endpoints tell MCP clients how to authenticate. Auth0 is the
//! authorization server; we just advertise its endpoints.

use axum::{extract::State, response::IntoResponse, Json};
use serde::Serialize;
use std::sync::Arc;

use crate::router::McpAppState;

/// RFC 9728 — Protected Resource Metadata.
///
/// Tells MCP clients which authorization server issues tokens for this
/// resource and how to present credentials.
#[derive(Serialize)]
struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    bearer_methods_supported: Vec<&'static str>,
    scopes_supported: Vec<&'static str>,
}

/// RFC 8414 — Authorization Server Metadata.
///
/// Returns Auth0's OAuth endpoints so MCP clients can perform the
/// authorization code + PKCE flow.
#[derive(Serialize)]
struct AuthorizationServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    scopes_supported: Vec<&'static str>,
    response_types_supported: Vec<&'static str>,
    grant_types_supported: Vec<&'static str>,
    code_challenge_methods_supported: Vec<&'static str>,
    resource: String,
}

/// `GET /.well-known/oauth-protected-resource`
pub async fn oauth_protected_resource(
    State(state): State<Arc<McpAppState>>,
) -> impl IntoResponse {
    let base = &state.mcp_config.mcp_base_url;
    let auth0 = state.mcp_config.auth0_domain.trim_end_matches('/');

    Json(ProtectedResourceMetadata {
        resource: format!("{base}/"),
        authorization_servers: vec![format!("{auth0}/")],
        bearer_methods_supported: vec!["header"],
        scopes_supported: vec!["openid", "profile", "email"],
    })
}

/// `GET /.well-known/oauth-authorization-server`
pub async fn oauth_authorization_server(
    State(state): State<Arc<McpAppState>>,
) -> impl IntoResponse {
    let auth0 = state.mcp_config.auth0_domain.trim_end_matches('/');

    Json(AuthorizationServerMetadata {
        issuer: format!("{auth0}/"),
        authorization_endpoint: format!("{auth0}/authorize"),
        token_endpoint: format!("{auth0}/oauth/token"),
        scopes_supported: vec!["openid", "profile", "email"],
        response_types_supported: vec!["code"],
        grant_types_supported: vec!["authorization_code", "refresh_token"],
        code_challenge_methods_supported: vec!["S256"],
        resource: state.mcp_config.mcp_audience.clone(),
    })
}
