//! OAuth2 Authorization Code + PKCE login flow with local callback server.
//!
//! 1. Generate PKCE code_verifier and code_challenge
//! 2. Open browser to provider's /authorize endpoint with
//!    redirect_uri pointing to the callback relay and state={port}
//! 3. Provider redirects to temperkb.io, which relays ?code= to localhost:{port}
//! 4. Exchange authorization code for tokens at /oauth/token
//! 5. Persist tokens to ~/.config/temper/auth.json

use chrono::Utc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

use crate::auth::{self, StoredAuth};
use crate::error::{ClientError, Result};

/// Provider-agnostic OAuth2 PKCE configuration.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// Authorization endpoint (e.g., `https://temperkb.us.auth0.com/authorize`)
    pub authorize_url: String,
    /// Token endpoint (e.g., `https://temperkb.us.auth0.com/oauth/token`)
    pub token_url: String,
    /// OAuth2 client ID
    pub client_id: String,
    /// API audience (sent as `audience` parameter)
    pub audience: Option<String>,
    /// Callback relay URL that forwards the authorization code to the CLI's localhost server.
    /// The CLI port is passed via the OAuth2 `state` parameter.
    pub callback_url: String,
    /// OAuth2 scopes (e.g., `["openid", "profile", "email", "offline_access"]`)
    pub scopes: Vec<String>,
}

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

/// Generate a PKCE code_verifier and its S256 code_challenge.
pub fn generate_pkce_pair() -> (String, String) {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use sha2::{Digest, Sha256};

    // 32 random bytes -> 43-character base64url string
    let random_bytes: [u8; 32] = rand::random();
    let verifier = URL_SAFE_NO_PAD.encode(random_bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    (verifier, challenge)
}

/// Build the full authorization URL with PKCE parameters.
///
/// Uses `CLI_CALLBACK_URL` as the redirect_uri and passes the localhost port
/// via the OAuth2 `state` parameter. The callback relay on temperkb.io reads
/// the port from `state` and redirects the authorization code to localhost.
pub fn build_authorize_url(
    config: &OAuthConfig,
    port: u16,
    code_challenge: &str,
) -> crate::error::Result<String> {
    let scope = config.scopes.join(" ");

    // `authorize_url` comes from user config (config.toml / provider) — a malformed value is a
    // configuration fault, not a programming bug, so propagate it instead of panicking.
    let mut url = url::Url::parse(&config.authorize_url).map_err(|e| {
        crate::error::ClientError::NotConfigured(format!(
            "authorize_url is not a valid URL ({:?}): {e}",
            config.authorize_url
        ))
    })?;

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &config.callback_url)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &port.to_string())
        .append_pair("scope", &scope);

    if let Some(audience) = &config.audience {
        url.query_pairs_mut().append_pair("audience", audience);
    }

    Ok(url.to_string())
}

// ---------------------------------------------------------------------------
// Token exchange
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[expect(
        dead_code,
        reason = "deserialized from the token response but unused — the access token is what we persist and decode for claims"
    )]
    id_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

/// Exchange an authorization code for tokens at the token endpoint.
async fn exchange_code(
    config: &OAuthConfig,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post(&config.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
            ("client_id", &config.client_id),
        ])
        .send()
        .await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ClientError::Other(format!("Token exchange failed: {body}")));
    }

    let tokens: TokenResponse = resp.json().await?;
    Ok(tokens)
}

// ---------------------------------------------------------------------------
// Login flow
// ---------------------------------------------------------------------------

pub async fn login(config: &OAuthConfig, store: &dyn auth::TokenStore) -> Result<StoredAuth> {
    // An empty callback URL produces an empty `redirect_uri` in the authorize
    // request, which Auth0 rejects with an opaque "Oops, something went wrong"
    // page. Catch it before opening a browser so the user gets an actionable
    // message. This is the unconfigured-cloud regression: the callback default
    // is empty until `temper init` derives one from the instance URL.
    if config.callback_url.is_empty() {
        return Err(ClientError::NotConfigured(
            "OAuth callback URL is not configured — run `temper init`".to_string(),
        ));
    }

    let (code_verifier, code_challenge) = generate_pkce_pair();

    // Bind to a random port on localhost.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    debug!(port, "Callback server listening");

    // Build authorization URL and open browser.
    // The redirect_uri points to temperkb.io which relays the code to localhost.
    let auth_url = build_authorize_url(config, port, &code_challenge)?;

    info!("Opening browser for authentication...");
    open::that(&auth_url)
        .map_err(|e| ClientError::Other(format!("failed to open browser: {e}")))?;

    // Wait for the callback with the authorization code.
    let code = wait_for_code(&listener).await?;

    debug!("Authorization code received, exchanging for tokens...");

    // Exchange code for tokens. The redirect_uri must match what was sent to /authorize.
    let tokens = exchange_code(config, &code, &code_verifier, &config.callback_url).await?;

    // Decode claims from the access token.
    let claims = auth::parse_jwt_claims(&tokens.access_token)?;

    let expires_at = if let Some(exp) = tokens.expires_in {
        Utc::now() + chrono::Duration::seconds(exp as i64)
    } else {
        claims.expires_at
    };

    let device_id = auth::load_or_create_device_id();

    let stored = StoredAuth {
        provider: auth::Provider::auth0(auth::default_auth0_domain()),
        access_token: tokens.access_token.into(),
        refresh_token: tokens.refresh_token.map(Into::into),
        expires_at,
        profile_id: claims.profile_id,
        device_id: Some(device_id),
    };

    store.save(&stored)?;
    info!("Authentication successful — token saved");

    Ok(stored)
}

/// Success page shown in the browser tab once the authorization code arrives.
const SUCCESS_HTML: &str = "<!DOCTYPE html><html><body>\
    <h2>Authentication successful!</h2>\
    <p>You can close this tab and return to the terminal.</p>\
    </body></html>";

/// Parsed outcome of an OAuth2 callback request's query string.
enum CallbackOutcome {
    /// Authorization code present — login can proceed.
    Code(String),
    /// Provider returned an error (`error` + optional `error_description`).
    Error { error: String, description: String },
    /// Neither code nor error — a stray request (favicon etc.); keep waiting.
    Pending,
}

/// Wait for the OAuth2 callback with an authorization code.
///
/// Orchestrates the per-connection phases: accept (bounded by the login
/// deadline), read + parse the request line, then route the callback to a
/// code, an error, or a keep-waiting outcome — writing a browser-facing
/// response on each path.
async fn wait_for_code(listener: &TcpListener) -> Result<String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);

    loop {
        let Some(mut stream) = accept_connection(listener, deadline).await? else {
            continue;
        };

        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);

        let (method, path) = parse_request_line(&request);
        debug!(method, path, "Received request");

        let is_callback = method == "GET"
            && (path.starts_with("/callback") || path.starts_with("/?") || path == "/");
        if !is_callback {
            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(response.as_bytes()).await;
            continue;
        }

        match extract_oauth_code(path)? {
            CallbackOutcome::Error { error, description } => {
                let html = format!(
                    "<!DOCTYPE html><html><body>\
                    <h2>Authentication Failed</h2>\
                    <p>{error}: {description}</p>\
                    </body></html>"
                );
                write_html_response(&mut stream, &html).await;
                return Err(ClientError::Other(format!(
                    "OAuth error: {error} — {description}"
                )));
            }
            CallbackOutcome::Code(code) => {
                write_html_response(&mut stream, SUCCESS_HTML).await;
                return Ok(code);
            }
            CallbackOutcome::Pending => {
                // No code in callback — keep waiting (might be favicon request etc.)
                write_html_response(&mut stream, SUCCESS_HTML).await;
            }
        }
    }
}

/// Accept the next inbound connection, honoring the overall login deadline.
///
/// Returns `Ok(Some(stream))` on a fresh connection, `Ok(None)` on a
/// recoverable accept error (caller should retry), or `Err` once the deadline
/// passes.
async fn accept_connection(
    listener: &TcpListener,
    deadline: tokio::time::Instant,
) -> Result<Option<TcpStream>> {
    match tokio::time::timeout_at(deadline, listener.accept()).await {
        Ok(Ok((stream, _))) => Ok(Some(stream)),
        Ok(Err(e)) => {
            warn!("accept error: {e}");
            Ok(None)
        }
        Err(_) => Err(ClientError::Other("authentication timed out (120s)".into())),
    }
}

/// Split the HTTP request line (`GET /path HTTP/1.1`) into method and path.
fn parse_request_line(request: &str) -> (&str, &str) {
    let first_line = request.lines().next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    (method, path)
}

/// Parse the callback `path`'s query string into a [`CallbackOutcome`].
///
/// An `error` parameter takes precedence over a `code`, matching the provider
/// contract (a callback never carries both meaningfully).
fn extract_oauth_code(path: &str) -> Result<CallbackOutcome> {
    let url = url::Url::parse(&format!("http://localhost{path}"))
        .map_err(|e| ClientError::Other(format!("parse error: {e}")))?;

    let find = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    };

    if let Some(error) = find("error") {
        let description = find("error_description").unwrap_or_default();
        return Ok(CallbackOutcome::Error { error, description });
    }

    Ok(match find("code") {
        Some(code) => CallbackOutcome::Code(code),
        None => CallbackOutcome::Pending,
    })
}

/// Write a `200 OK` HTML response to the callback stream.
///
/// Write errors are ignored: the browser tab is cosmetic, and the
/// authorization outcome is already decided by the time we respond.
async fn write_html_response(stream: &mut TcpStream, html: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_valid_s256() {
        let (verifier, challenge) = generate_pkce_pair();
        assert!(verifier.len() >= 43 && verifier.len() <= 128);
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        use sha2::{Digest, Sha256};
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
    }

    #[test]
    fn authorize_url_contains_required_params() {
        let config = OAuthConfig {
            authorize_url: "https://temperkb.us.auth0.com/authorize".to_string(),
            token_url: "https://temperkb.us.auth0.com/oauth/token".to_string(),
            client_id: "test-client-id".to_string(),
            audience: Some("https://temperkb.io/api".to_string()),
            callback_url: "https://temperkb.io/api/auth/cli-callback".to_string(),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ],
        };
        let (_verifier, challenge) = generate_pkce_pair();
        let url = build_authorize_url(&config, 12345, &challenge).unwrap();
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("code_challenge={challenge}")));
        assert!(url.contains("audience="));
        // redirect_uri goes to temperkb.io, port is in state
        assert!(url.contains("redirect_uri=https%3A%2F%2Ftemperkb.io%2Fapi%2Fauth%2Fcli-callback"));
        assert!(url.contains("state=12345"));
    }

    #[test]
    fn authorize_url_without_audience() {
        let config = OAuthConfig {
            authorize_url: "https://example.com/authorize".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "test".to_string(),
            audience: None,
            callback_url: "https://example.com/callback".to_string(),
            scopes: vec!["openid".to_string()],
        };
        let (_verifier, challenge) = generate_pkce_pair();
        let url = build_authorize_url(&config, 9999, &challenge).unwrap();
        assert!(!url.contains("audience="));
    }

    #[tokio::test]
    async fn login_with_empty_callback_returns_not_configured() {
        // Regression: an unconfigured callback URL must fail fast with an
        // actionable message rather than opening a browser to a broken Auth0
        // authorize URL (empty redirect_uri → Auth0 "Oops" page).
        let config = OAuthConfig {
            authorize_url: "https://example.com/authorize".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "test".to_string(),
            audience: None,
            callback_url: String::new(),
            scopes: vec!["openid".to_string()],
        };
        let store = auth::MemoryTokenStore::empty();
        let err = login(&config, &store)
            .await
            .expect_err("empty callback URL must error before opening a browser");
        match err {
            ClientError::NotConfigured(msg) => {
                assert!(msg.contains("temper init"), "got: {msg}");
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
    }
}
