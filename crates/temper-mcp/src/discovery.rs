//! OAuth well-known discovery endpoints and dynamic client registration.
//!
//! These endpoints tell MCP clients how to authenticate. Auth0 is the
//! authorization server; we advertise its endpoints and provide a thin
//! registration endpoint that returns our pre-registered client_id.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
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
/// authorization code + PKCE flow. Includes a `registration_endpoint`
/// pointing to our thin DCR proxy.
#[derive(Serialize)]
struct AuthorizationServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: String,
    scopes_supported: Vec<&'static str>,
    response_types_supported: Vec<&'static str>,
    grant_types_supported: Vec<&'static str>,
    code_challenge_methods_supported: Vec<&'static str>,
    resource: String,
}

/// `GET /.well-known/oauth-protected-resource`
pub async fn oauth_protected_resource(State(state): State<Arc<McpAppState>>) -> impl IntoResponse {
    let base = &state.mcp_config.mcp_base_url;

    Json(ProtectedResourceMetadata {
        resource: format!("{base}/"),
        authorization_servers: vec![format!("{base}/")],
        bearer_methods_supported: vec!["header"],
        scopes_supported: vec!["openid", "profile", "email", "offline_access"],
    })
}

/// `GET /.well-known/oauth-authorization-server`
pub async fn oauth_authorization_server(
    State(state): State<Arc<McpAppState>>,
) -> impl IntoResponse {
    let base = &state.mcp_config.mcp_base_url;
    let auth0 = state.mcp_config.auth0_domain.trim_end_matches('/');

    Json(AuthorizationServerMetadata {
        issuer: format!("{auth0}/"),
        authorization_endpoint: format!("{auth0}/authorize"),
        token_endpoint: format!("{auth0}/oauth/token"),
        registration_endpoint: format!("{base}/oauth/register"),
        scopes_supported: vec!["openid", "profile", "email", "offline_access"],
        response_types_supported: vec!["code"],
        grant_types_supported: vec!["authorization_code", "refresh_token"],
        code_challenge_methods_supported: vec!["S256"],
        resource: state.mcp_config.mcp_audience.clone(),
    })
}

// ── Dynamic Client Registration (thin proxy) ──────────────────────────

/// RFC 7591 — Client registration request (subset).
/// We accept whatever the MCP client sends but only use a few fields
/// for the response. The actual Auth0 application is pre-registered.
#[derive(Debug, Deserialize)]
pub struct ClientRegistrationRequest {
    pub client_name: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
    // Accept and ignore any other fields the client sends.
}

/// RFC 7591 — Client registration response.
#[derive(Serialize)]
struct ClientRegistrationResponse {
    client_id: String,
    client_name: String,
    redirect_uris: Vec<String>,
    grant_types: Vec<&'static str>,
    response_types: Vec<&'static str>,
    token_endpoint_auth_method: &'static str,
}

/// `POST /oauth/register` — Dynamic Client Registration endpoint.
///
/// Returns the pre-registered Auth0 application's `client_id` to any
/// MCP client that requests registration. This gives clients like
/// Claude Desktop the seamless connector experience (no manual
/// client_id entry) without opening Auth0's native DCR endpoint.
///
/// Only redirect URIs listed in `mcp-server.toml` are echoed back.
/// Returns 503 if `MCP_CLIENT_ID` is not configured.
pub async fn register_client(
    State(state): State<Arc<McpAppState>>,
    Json(request): Json<ClientRegistrationRequest>,
) -> impl IntoResponse {
    let Some(ref client_id) = state.mcp_config.mcp_client_id else {
        tracing::warn!("DCR request received but MCP_CLIENT_ID is not configured");
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "temporarily_unavailable",
                "error_description": "Dynamic client registration is not configured"
            })),
        ));
    };

    let client_name = request
        .client_name
        .unwrap_or_else(|| "MCP Client".to_string());

    // Only echo back redirect URIs that are in our allowed list
    // (or localhost URIs when allow_localhost is enabled).
    let oauth = &state.mcp_config.oauth;
    let redirect_uris: Vec<String> = request
        .redirect_uris
        .unwrap_or_default()
        .into_iter()
        .filter(|uri| {
            oauth.redirect_uris.contains(uri) || (oauth.allow_localhost && is_localhost_uri(uri))
        })
        .collect();

    tracing::info!(
        client_name = %client_name,
        redirect_uris = ?redirect_uris,
        "MCP dynamic client registration (returning static client_id)"
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!(ClientRegistrationResponse {
            client_id: client_id.clone(),
            client_name,
            redirect_uris,
            grant_types: vec!["authorization_code", "refresh_token"],
            response_types: vec!["code"],
            token_endpoint_auth_method: "none",
        })),
    ))
}

/// Returns true if the URI is an `http://localhost` or `http://127.0.0.1` callback.
/// These are used by desktop/CLI MCP clients that run local OAuth servers.
fn is_localhost_uri(uri: &str) -> bool {
    uri.starts_with("http://localhost") || uri.starts_with("http://127.0.0.1")
}
