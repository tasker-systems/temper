//! OAuth2 Authorization Code + PKCE login flow with local callback server.
//!
//! 1. Generate PKCE code_verifier and code_challenge
//! 2. Open browser to provider's /authorize endpoint with
//!    redirect_uri pointing to the callback relay and state={port}
//! 3. Provider redirects to temperkb.io, which relays ?code= to localhost:{port}
//! 4. Exchange authorization code for tokens at /oauth/token
//! 5. Persist tokens to ~/.config/temper/auth.json

use chrono::{DateTime, Utc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
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
pub fn build_authorize_url(config: &OAuthConfig, port: u16, code_challenge: &str) -> String {
    let scope = config.scopes.join(" ");

    let mut url =
        url::Url::parse(&config.authorize_url).expect("authorize_url must be a valid URL");

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

    url.to_string()
}

// ---------------------------------------------------------------------------
// Token exchange
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[allow(dead_code)]
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

pub async fn login(config: &OAuthConfig) -> Result<StoredAuth> {
    let (code_verifier, code_challenge) = generate_pkce_pair();

    // Bind to a random port on localhost.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    debug!(port, "Callback server listening");

    // Build authorization URL and open browser.
    // The redirect_uri points to temperkb.io which relays the code to localhost.
    let auth_url = build_authorize_url(config, port, &code_challenge);

    info!("Opening browser for authentication...");
    open::that(&auth_url)
        .map_err(|e| ClientError::Other(format!("failed to open browser: {e}")))?;

    // Wait for the callback with the authorization code.
    let code = wait_for_code(&listener).await?;

    debug!("Authorization code received, exchanging for tokens...");

    // Exchange code for tokens. The redirect_uri must match what was sent to /authorize.
    let tokens = exchange_code(config, &code, &code_verifier, &config.callback_url).await?;

    // Decode claims from the access token.
    let claims = decode_jwt_claims(&tokens.access_token)?;

    let expires_at = if let Some(exp) = tokens.expires_in {
        Utc::now() + chrono::Duration::seconds(exp as i64)
    } else {
        claims.expires_at
    };

    let device_id = auth::load_or_create_device_id();

    let stored = StoredAuth {
        provider: "auth0".to_owned(),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at,
        profile_id: claims.subject,
        device_id: Some(device_id),
    };

    auth::save_auth(&stored)?;
    info!("Authentication successful — token saved");

    Ok(stored)
}

/// Wait for the OAuth2 callback with an authorization code.
async fn wait_for_code(listener: &TcpListener) -> Result<String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);

    loop {
        let accept = tokio::time::timeout_at(deadline, listener.accept()).await;

        let (mut stream, _) = match accept {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => {
                warn!("accept error: {e}");
                continue;
            }
            Err(_) => {
                return Err(ClientError::Other("authentication timed out (120s)".into()));
            }
        };

        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await?;
        let request_str = String::from_utf8_lossy(&buf[..n]);

        let first_line = request_str.lines().next().unwrap_or("");
        let parts: Vec<&str> = first_line.split_whitespace().collect();
        let method = parts.first().copied().unwrap_or("");
        let path = parts.get(1).copied().unwrap_or("");

        debug!(method, path, "Received request");

        if method == "GET"
            && (path.starts_with("/callback") || path.starts_with("/?") || path == "/")
        {
            let full_url = url::Url::parse(&format!("http://localhost{path}"))
                .map_err(|e| ClientError::Other(format!("parse error: {e}")))?;

            let code = full_url
                .query_pairs()
                .find(|(k, _)| k == "code")
                .map(|(_, v)| v.into_owned());

            // Check for error response from provider
            let error = full_url
                .query_pairs()
                .find(|(k, _)| k == "error")
                .map(|(_, v)| v.into_owned());

            if let Some(err) = error {
                let desc = full_url
                    .query_pairs()
                    .find(|(k, _)| k == "error_description")
                    .map(|(_, v)| v.into_owned())
                    .unwrap_or_default();

                let html = format!(
                    "<!DOCTYPE html><html><body>\
                    <h2>Authentication Failed</h2>\
                    <p>{err}: {desc}</p>\
                    </body></html>"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = stream.write_all(response.as_bytes()).await;

                return Err(ClientError::Other(format!("OAuth error: {err} — {desc}")));
            }

            // Send success response
            let html = "<!DOCTYPE html><html><body>\
                <h2>Authentication successful!</h2>\
                <p>You can close this tab and return to the terminal.</p>\
                </body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = stream.write_all(response.as_bytes()).await;

            if let Some(c) = code {
                return Ok(c);
            }
            // No code in callback — keep waiting (might be favicon request etc.)
        } else {
            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(response.as_bytes()).await;
        }
    }
}

struct JwtClaims {
    expires_at: DateTime<Utc>,
    subject: Option<uuid::Uuid>,
}

fn decode_jwt_claims(jwt: &str) -> Result<JwtClaims> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err(ClientError::Other("invalid JWT format".into()));
    }

    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| ClientError::Other(format!("JWT decode error: {e}")))?;

    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes)?;

    let exp = payload
        .get("exp")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ClientError::Other("JWT missing exp claim".into()))?;

    let expires_at = DateTime::from_timestamp(exp, 0)
        .ok_or_else(|| ClientError::Other("invalid exp timestamp".into()))?;

    // Auth0 sub claims are strings like "google-oauth2|12345", not UUIDs.
    // Try to parse as UUID but don't fail if it isn't one.
    let subject = payload
        .get("sub")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    Ok(JwtClaims {
        expires_at,
        subject,
    })
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
        let url = build_authorize_url(&config, 12345, &challenge);
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
        let url = build_authorize_url(&config, 9999, &challenge);
        assert!(!url.contains("audience="));
    }

    #[test]
    fn decode_jwt_extracts_exp_and_sub() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD
            .encode(r#"{"sub":"418200cc-e77a-496f-b5cc-eb4228e0e828","exp":1711800000}"#);
        let sig = URL_SAFE_NO_PAD.encode("fakesig");
        let jwt = format!("{header}.{payload}.{sig}");

        let claims = decode_jwt_claims(&jwt).unwrap();
        assert!(claims.subject.is_some());
        assert_eq!(
            claims.subject.unwrap().to_string(),
            "418200cc-e77a-496f-b5cc-eb4228e0e828"
        );
        assert_eq!(claims.expires_at.timestamp(), 1711800000);
    }

    #[test]
    fn decode_jwt_handles_auth0_sub_format() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#);
        let payload =
            URL_SAFE_NO_PAD.encode(r#"{"sub":"google-oauth2|123456789","exp":1711800000}"#);
        let sig = URL_SAFE_NO_PAD.encode("fakesig");
        let jwt = format!("{header}.{payload}.{sig}");

        let claims = decode_jwt_claims(&jwt).unwrap();
        // Auth0 sub is not a UUID — should be None, not an error
        assert!(claims.subject.is_none());
        assert_eq!(claims.expires_at.timestamp(), 1711800000);
    }

    #[test]
    fn decode_jwt_rejects_malformed() {
        assert!(decode_jwt_claims("not.a.valid-jwt").is_err());
        assert!(decode_jwt_claims("only-one-part").is_err());
    }
}
