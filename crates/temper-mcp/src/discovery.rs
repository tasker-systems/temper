//! OAuth protected-resource metadata and dynamic client registration.
//!
//! These endpoints tell MCP clients how to authenticate: the RFC 9728
//! protected-resource metadata and a registration endpoint that serves two
//! populations — the thin static-client echo MCP clients have always gotten, and (on
//! AS-mode instances with `AS_CONNECT_DCR` enabled) a real RFC 7591 mint for Vercel
//! Connect connectors. The RFC 8414 authorization-server metadata
//! (`/.well-known/oauth-authorization-server`) is served by the temper-cloud
//! AS layer instead, so a single handler can advertise either the Temper AS
//! (SAML instances) or Auth0 (legacy instances) from one shared deployment.

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

/// Build RFC 9728 protected-resource metadata for the given server base URL.
///
/// `offline_access` is advertised so conformant MCP clients request it
/// during the authorization code flow, prompting Auth0 to issue a refresh
/// token (avoids a full re-auth on every access token expiry).
fn protected_resource_metadata(base: &str) -> ProtectedResourceMetadata {
    ProtectedResourceMetadata {
        resource: format!("{base}/"),
        authorization_servers: vec![format!("{base}/")],
        bearer_methods_supported: vec!["header"],
        scopes_supported: vec!["openid", "profile", "email", "offline_access"],
    }
}

/// `GET /.well-known/oauth-protected-resource`
pub async fn oauth_protected_resource(State(state): State<Arc<McpAppState>>) -> impl IntoResponse {
    Json(protected_resource_metadata(&state.mcp_config.mcp_base_url))
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

/// RFC 7591 §3.2.2 — Client registration error response.
#[derive(Serialize)]
struct OAuthErrorResponse {
    error: &'static str,
    error_description: &'static str,
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

/// RFC 7591 §3.2.1 — Client information response for a MINTED client (the Connect arm).
///
/// Unlike [`ClientRegistrationResponse`] (an echo of a pre-registered static client), this
/// carries the fields only a real registration can: a server-generated secret, its issue
/// time, and `client_secret_expires_at` (RFC 7591 §3.2.1: `0` means no expiry — rotation
/// exists through the machine-client machinery, and Connect treats 0 as "does not expire").
#[derive(Serialize)]
struct MintedClientRegistrationResponse {
    client_id: String,
    client_secret: String,
    client_id_issued_at: i64,
    client_secret_expires_at: i64,
    client_name: String,
    redirect_uris: Vec<String>,
    /// `client_credentials` is what makes app subjects possible — round 1 of the through-path
    /// measurement settled that the registration response, not the 8414 document, decides
    /// `supportedSubjectTypes`, and the proxy's grant list (which omitted it) is exactly why
    /// Connect concluded `["user"]`. `authorization_code`/`refresh_token` serve the user-OAuth
    /// installation arm, which uses this same client and its callback redirect.
    grant_types: Vec<&'static str>,
    response_types: Vec<&'static str>,
    /// The honest method the temper AS declares and `verifyMachineSecret` accepts (Basic
    /// preferred, RFC 6749 §2.3.1). Not `none` — this client carries a secret.
    token_endpoint_auth_method: &'static str,
}

/// The redirect URI that engages the Connect arm. Vercel Connect's custom-OAuth providers
/// register with exactly this callback; no MCP client proposes it, which is what makes it a
/// safe discriminator between the two populations this one endpoint serves.
const CONNECT_CALLBACK: &str = "https://connect.vercel.com/callback";

/// True iff the proposal includes Vercel Connect's callback — the marker of a
/// Connect-shaped registration.
fn is_connect_shaped(redirect_uris: &[String]) -> bool {
    redirect_uris.iter().any(|uri| uri == CONNECT_CALLBACK)
}

/// `POST /oauth/register` — Dynamic Client Registration endpoint.
///
/// Two populations share this endpoint, discriminated by the one thing that cannot lie about
/// which is asking: the proposed redirect URIs.
///
/// - **MCP clients** (Claude Desktop/Code) propose loopback or desktop callbacks. They get the
///   thin static-client echo this handler has always been: the pre-registered Auth0/MCP
///   application's `client_id`, no secret, `token_endpoint_auth_method: "none"`. Only
///   redirect URIs listed in `mcp-server.toml` (or localhost, when allowed) are echoed back;
///   503 if `MCP_CLIENT_ID` is not configured.
///
/// - **Vercel Connect** proposes exactly `https://connect.vercel.com/callback`. On an AS-mode
///   instance with `AS_CONNECT_DCR` enabled it gets a REAL registration:
///   `machine_registration_service::issue_connect` (temper-services) mints a
///   `kb_machine_clients` row
///   (`issuer='temper'`) with a server-generated secret that verifies at `/oauth/token`'s
///   `client_credentials` arm. The minted credential is born with empty reach and denied
///   standing — containment by construction; a tenant admin confers authority later through
///   the ordinary standing/grant machinery. Redirect URIs are validated (any URI other than
///   the Connect callback refuses the registration, RFC 7591 §3.2.2 `invalid_redirect_uri`)
///   and echoed, **never persisted** — `config.rs`'s load-bearing invariant holds that a
///   registration which persists client-supplied redirect URIs reintroduces the
///   redirect-to-code-capture chain, and open-redirect protection stays enforced at
///   `/oauth/authorize` against `AS_CLIENTS`.
///
///   With the gate off (the default, including every non-AS instance), a Connect-shaped
///   request is refused with 503 rather than falling through to the static echo — the echo is
///   the wrong answer for a client that will never authenticate as it, and a silent wrong
///   answer here reads downstream as "app subjects unsupported" (round 1's exact finding).
pub async fn register_client(
    State(state): State<Arc<McpAppState>>,
    Json(request): Json<ClientRegistrationRequest>,
) -> axum::response::Response {
    let proposed = request.redirect_uris.clone().unwrap_or_default();

    if is_connect_shaped(&proposed) {
        return register_connect_client(state, request, proposed)
            .await
            .into_response();
    }

    let Some(ref client_id) = state.mcp_config.mcp_client_id else {
        tracing::warn!("DCR request received but MCP_CLIENT_ID is not configured");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(OAuthErrorResponse {
                error: "temporarily_unavailable",
                error_description: "Dynamic client registration is not configured",
            }),
        )
            .into_response();
    };

    let client_name = request
        .client_name
        .unwrap_or_else(|| "MCP Client".to_string());

    // Only echo back redirect URIs that are in our allowed list
    // (or localhost URIs when allow_localhost is enabled).
    let oauth = &state.mcp_config.oauth;
    let redirect_uris: Vec<String> = proposed
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

    (
        StatusCode::CREATED,
        Json(ClientRegistrationResponse {
            client_id: client_id.clone(),
            client_name,
            redirect_uris,
            grant_types: vec!["authorization_code", "refresh_token"],
            response_types: vec!["code"],
            token_endpoint_auth_method: "none",
        }),
    )
        .into_response()
}

/// The Connect arm of [`register_client`]: gate, validate, mint.
async fn register_connect_client(
    state: Arc<McpAppState>,
    request: ClientRegistrationRequest,
    proposed: Vec<String>,
) -> Result<
    (StatusCode, Json<MintedClientRegistrationResponse>),
    (StatusCode, Json<OAuthErrorResponse>),
> {
    if !state.mcp_config.connect_dcr_ready() {
        tracing::warn!(
            "Connect-shaped DCR request refused: AS_CONNECT_DCR is not enabled on this instance"
        );
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(OAuthErrorResponse {
                error: "temporarily_unavailable",
                error_description: "Connect dynamic client registration is not enabled",
            }),
        ));
    }

    // The discriminator was the callback's PRESENCE; the validation is its EXCLUSIVITY. A
    // proposal mixing the Connect callback with any other URI is not a shape Connect sends —
    // refusing it (rather than filtering, as the MCP arm does) keeps the arm's accept-set
    // honest and fail-loud per RFC 7591 §3.2.2.
    if proposed.iter().any(|uri| uri != CONNECT_CALLBACK) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(OAuthErrorResponse {
                error: "invalid_redirect_uri",
                error_description:
                    "only https://connect.vercel.com/callback is accepted for this registration",
            }),
        ));
    }

    let client_name = request
        .client_name
        .unwrap_or_else(|| "Vercel Connect".to_string());

    let cred = match temper_services::services::machine_registration_service::issue_connect(
        &state.api_state.pool,
        &client_name,
    )
    .await
    {
        Ok(cred) => cred,
        Err(e) => {
            tracing::error!(error = %e, "Connect DCR mint failed");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(OAuthErrorResponse {
                    error: "temporarily_unavailable",
                    error_description: "Client registration could not be completed",
                }),
            ));
        }
    };

    tracing::info!(
        client_id = %cred.client.client_id,
        client_name = %client_name,
        "Connect DCR: minted AS client (born denied, empty reach)"
    );

    Ok((
        StatusCode::CREATED,
        Json(MintedClientRegistrationResponse {
            client_id: cred.client.client_id,
            client_secret: cred.client_secret,
            client_id_issued_at: cred.client.created.timestamp(),
            client_secret_expires_at: 0,
            client_name,
            redirect_uris: proposed,
            grant_types: vec!["client_credentials", "authorization_code", "refresh_token"],
            response_types: vec!["code"],
            token_endpoint_auth_method: "client_secret_basic",
        }),
    ))
}

/// Returns true if the URI is an `http://localhost` or `http://127.0.0.1` callback.
/// These are used by desktop/CLI MCP clients that run local OAuth servers.
fn is_localhost_uri(uri: &str) -> bool {
    uri.starts_with("http://localhost") || uri.starts_with("http://127.0.0.1")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MCP clients only request scopes the server advertises. Without
    /// `offline_access` here, Auth0 never issues a refresh token and every
    /// access token expiry forces a full re-auth.
    #[test]
    fn protected_resource_metadata_advertises_offline_access() {
        let meta = protected_resource_metadata("https://temperkb.io");
        assert!(
            meta.scopes_supported.contains(&"offline_access"),
            "offline_access must be advertised: {:?}",
            meta.scopes_supported
        );
    }

    #[test]
    fn is_localhost_uri_accepts_loopback_callbacks() {
        assert!(is_localhost_uri("http://localhost:8080/callback"));
        assert!(is_localhost_uri("http://127.0.0.1:53682/callback"));
    }

    #[test]
    fn is_localhost_uri_rejects_remote_uris() {
        assert!(!is_localhost_uri("https://temperkb.io/callback"));
        assert!(!is_localhost_uri("https://localhost.evil.com/callback"));
    }

    /// The Connect arm's discriminator keys on the exact callback, nothing else.
    ///
    /// Prefix and scheme variants that a string-contains check would admit must not engage the
    /// mint — the arm's whole safety property is that only Connect proposes this URI.
    #[test]
    fn is_connect_shaped_keys_on_the_exact_callback() {
        let v = |uris: &[&str]| uris.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_connect_shaped(&v(&[
            "https://connect.vercel.com/callback"
        ])));
        assert!(is_connect_shaped(&v(&[
            "http://127.0.0.1:8080/callback",
            "https://connect.vercel.com/callback"
        ])));
        assert!(!is_connect_shaped(&v(&[
            "https://connect.vercel.com/callback/extra"
        ])));
        assert!(!is_connect_shaped(&v(&[
            "https://connect.vercel.com/callback.evil.com"
        ])));
        assert!(!is_connect_shaped(&[]));
    }
}
