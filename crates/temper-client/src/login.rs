//! OAuth2 PKCE login flow with local callback server.
//!
//! Opens the browser to the provider's authorize URL, spins up a one-shot
//! TCP listener on localhost to capture the redirect, exchanges the auth code
//! for tokens, and persists the result to `~/.config/temper/auth.json`.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, info};
use url::Url;

use crate::auth::{self, StoredAuth};
use crate::error::{ClientError, Result};

/// Configuration for the OAuth2 PKCE flow.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    pub scopes: Vec<String>,
}

/// OAuth2 token response — the fields we extract from the provider.
#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

/// Charset used for PKCE code_verifier (RFC 7636 Appendix B).
const PKCE_CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// Generate a random PKCE code_verifier of the given length.
///
/// Length must be 43..=128 per RFC 7636. Panics if out of range.
pub(crate) fn generate_code_verifier(len: usize) -> String {
    assert!(
        (43..=128).contains(&len),
        "code_verifier length must be 43..=128"
    );
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let idx = rng.gen_range(0..PKCE_CHARSET.len());
            PKCE_CHARSET[idx] as char
        })
        .collect()
}

/// Compute the S256 code_challenge from a code_verifier.
///
/// `code_challenge = BASE64URL(SHA256(code_verifier))` with no padding.
pub(crate) fn compute_code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Generate a random hex state parameter (32 hex chars = 16 random bytes).
pub(crate) fn generate_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill(&mut bytes);
    hex::encode(bytes)
}

// ---------------------------------------------------------------------------
// Login flow
// ---------------------------------------------------------------------------

/// Run the full OAuth2 PKCE login flow:
///
/// 1. Generate PKCE verifier + challenge and random state
/// 2. Bind a localhost callback server
/// 3. Open the browser to the authorize URL
/// 4. Wait for the redirect callback with the auth code
/// 5. Exchange the code for tokens
/// 6. Persist and return [`StoredAuth`]
pub async fn login(config: &OAuthConfig) -> Result<StoredAuth> {
    let code_verifier = generate_code_verifier(128);
    let code_challenge = compute_code_challenge(&code_verifier);
    let state = generate_state();

    // Bind to a random port on localhost.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    debug!(port, "PKCE callback server listening");

    // Build the authorize URL.
    let mut authorize_url = Url::parse(&config.authorize_url)
        .map_err(|e| ClientError::Other(format!("invalid authorize_url: {e}")))?;

    authorize_url
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", &config.scopes.join(" "))
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state);

    info!("Opening browser for authentication…");
    open::that(authorize_url.as_str())
        .map_err(|e| ClientError::Other(format!("failed to open browser: {e}")))?;

    // Accept exactly one connection and read the callback.
    let (mut stream, _addr) = listener.accept().await?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request_str = String::from_utf8_lossy(&buf[..n]);

    // Parse the first line: "GET /callback?code=...&state=... HTTP/1.1"
    let first_line = request_str
        .lines()
        .next()
        .ok_or_else(|| ClientError::Other("empty HTTP request".into()))?;

    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| ClientError::Other("malformed HTTP request line".into()))?;

    // Parse query params using a dummy base URL.
    let full_url = Url::parse(&format!("http://localhost{path}"))
        .map_err(|e| ClientError::Other(format!("failed to parse callback URL: {e}")))?;

    let params: std::collections::HashMap<String, String> = full_url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    // Verify state matches.
    let returned_state = params
        .get("state")
        .ok_or_else(|| ClientError::Other("missing state in callback".into()))?;

    if *returned_state != state {
        let body = "Authentication failed: state mismatch.";
        let response = format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes()).await;
        return Err(ClientError::Other("OAuth state mismatch".into()));
    }

    let code = params
        .get("code")
        .ok_or_else(|| ClientError::Other("missing code in callback".into()))?;

    // Send success response to the browser.
    let html = "<!DOCTYPE html><html><body><h2>Authentication successful!</h2>\
                <p>You can close this tab.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(response.as_bytes()).await;

    // Exchange the authorization code for tokens.
    let client = reqwest::Client::new();
    let resp = client
        .post(&config.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &redirect_uri),
            ("client_id", &config.client_id),
            ("code_verifier", &code_verifier),
        ])
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ClientError::Other(format!(
            "token exchange failed ({status}): {body}",
        )));
    }

    let tr: TokenResponse = resp.json().await?;
    let expires_in = tr.expires_in.unwrap_or(3600);
    let expires_at = Utc::now() + Duration::seconds(expires_in as i64);

    let stored = StoredAuth {
        provider: "oauth".to_owned(),
        access_token: tr.access_token,
        refresh_token: tr.refresh_token,
        expires_at,
        profile_id: None,
    };

    auth::save_auth(&stored)?;
    info!("Authentication successful — token saved");

    Ok(stored)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_verifier_length_and_charset() {
        let verifier = generate_code_verifier(43);
        assert_eq!(verifier.len(), 43);

        let verifier = generate_code_verifier(128);
        assert_eq!(verifier.len(), 128);

        // Every character must be in the PKCE charset.
        for ch in verifier.chars() {
            assert!(
                PKCE_CHARSET.contains(&(ch as u8)),
                "unexpected char in verifier: {ch:?}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "code_verifier length must be 43..=128")]
    fn code_verifier_rejects_too_short() {
        generate_code_verifier(42);
    }

    #[test]
    #[should_panic(expected = "code_verifier length must be 43..=128")]
    fn code_verifier_rejects_too_long() {
        generate_code_verifier(129);
    }

    #[test]
    fn code_challenge_known_test_vector() {
        // RFC 7636 Appendix B test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = compute_code_challenge(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn state_generation_length_and_hex() {
        let state = generate_state();
        assert_eq!(state.len(), 32, "state should be 32 hex chars");
        assert!(
            state.chars().all(|c| c.is_ascii_hexdigit()),
            "state should contain only hex digits"
        );
    }
}
