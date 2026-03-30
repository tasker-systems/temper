//! Neon Auth (Better Auth) login flow with local callback server.
//!
//! 1. POST to Neon Auth `/sign-in/social` to get a Google OAuth redirect URL
//! 2. Open the browser to that URL
//! 3. Spin up a localhost TCP server for the callback
//! 4. Neon Auth redirects back to localhost after Google sign-in
//! 5. Serve an HTML page that fetches `/auth/token` with `credentials: include`
//!    (cookies are first-party because localhost is the redirect target)
//! 6. The page POSTs the JWT back to the localhost server
//! 7. Persist the token to `~/.config/temper/auth.json`

use chrono::{DateTime, Utc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

use crate::auth::{self, StoredAuth};
use crate::error::{ClientError, Result};

/// Configuration for the Neon Auth social sign-in flow.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// Neon Auth base URL (e.g., `https://ep-xxx.neonauth.region.aws.neon.tech/neondb/auth`)
    pub authorize_url: String,
    /// Token URL — same as authorize_url for Better Auth (not used separately)
    pub token_url: String,
    /// Not used for Better Auth, kept for interface compat
    pub client_id: String,
    /// OAuth provider (e.g., "google")
    pub scopes: Vec<String>,
}

impl OAuthConfig {
    /// The Neon Auth base URL (stored in authorize_url for config compat).
    fn neon_auth_base(&self) -> &str {
        &self.authorize_url
    }

    /// The OAuth provider name. Defaults to "google" if scopes is empty.
    fn provider(&self) -> &str {
        self.scopes.first().map(|s| s.as_str()).unwrap_or("google")
    }
}

/// Response from Better Auth `/sign-in/social` endpoint.
#[derive(Debug, serde::Deserialize)]
struct SignInResponse {
    url: Option<String>,
    redirect: Option<bool>,
}

// ---------------------------------------------------------------------------
// Login flow
// ---------------------------------------------------------------------------

/// Run the Neon Auth login flow:
///
/// 1. Bind a localhost callback server on a random port
/// 2. POST to Neon Auth `/sign-in/social` to get the Google OAuth URL
/// 3. Open the browser to that URL
/// 4. Wait for the redirect callback from Neon Auth
/// 5. Serve an HTML page that fetches the JWT using browser cookies
/// 6. Wait for the page to POST the JWT back
/// 7. Persist and return [`StoredAuth`]
pub async fn login(config: &OAuthConfig) -> Result<StoredAuth> {
    let neon_auth = config.neon_auth_base();
    let provider = config.provider();

    // Bind to a random port on localhost.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    // Use temperkb.io as the callback so session cookies are forwarded
    // server-side. The Vercel endpoint fetches the JWT and redirects
    // back to localhost with ?jwt=<token>.
    let callback_url = format!("https://temperkb.io/api/auth-callback?cli_port={port}");

    debug!(port, "Callback server listening");

    // POST to Neon Auth to initiate social sign-in.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| ClientError::Other(format!("http client: {e}")))?;

    let resp = client
        .post(format!("{neon_auth}/sign-in/social"))
        .header("Content-Type", "application/json")
        .header("Origin", "https://temperkb.io")
        .json(&serde_json::json!({
            "provider": provider,
            "callbackURL": callback_url,
        }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ClientError::Other(format!(
            "Neon Auth sign-in failed: {body}"
        )));
    }

    let sign_in: SignInResponse = resp.json().await?;
    let auth_url = sign_in
        .url
        .ok_or_else(|| ClientError::Other("no redirect URL from Neon Auth".into()))?;

    info!("Opening browser for authentication...");
    open::that(&auth_url)
        .map_err(|e| ClientError::Other(format!("failed to open browser: {e}")))?;

    // Now we need to handle two requests:
    // 1. The callback redirect from Neon Auth (GET /callback?neon_auth_session_verifier=...)
    //    → serve HTML page that fetches JWT
    // 2. The JWT POST from the HTML page (POST /token with JWT in body)
    //    → capture and save

    let mut jwt: Option<String> = None;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);

    while jwt.is_none() {
        let accept = tokio::time::timeout_at(deadline, listener.accept()).await;

        let (mut stream, _addr) = match accept {
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

        if method == "GET" && path.starts_with("/token") {
            // The Vercel callback redirected here with ?jwt=<token>
            let full_url = url::Url::parse(&format!("http://localhost{path}"))
                .map_err(|e| ClientError::Other(format!("parse error: {e}")))?;
            let token = full_url
                .query_pairs()
                .find(|(k, _)| k == "jwt")
                .map(|(_, v)| v.into_owned());

            if let Some(token) = token {
                if token.starts_with("eyJ") {
                    jwt = Some(token);
                    let html = "<!DOCTYPE html><html><body><h2>Authenticated!</h2><p>You can close this tab.</p></body></html>";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html.len(), html
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                } else {
                    let html = format!("<!DOCTYPE html><html><body><h2>Error</h2><pre>Invalid token format</pre></body></html>");
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html.len(), html
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                }
            }
        } else if method == "GET" && path.starts_with("/callback") {
            // Legacy: direct callback without JWT — show waiting message
            let html = "<!DOCTYPE html><html><body><h2>Waiting for authentication...</h2><p>Processing your sign-in. This page will update automatically.</p></body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(), html
            );
            let _ = stream.write_all(response.as_bytes()).await;
        } else {
            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(response.as_bytes()).await;
        }
    }

    let jwt = jwt.expect("jwt should be set after loop");

    // Decode JWT to extract expiry and subject
    let claims = decode_jwt_claims(&jwt)?;

    let stored = StoredAuth {
        provider: provider.to_owned(),
        access_token: jwt,
        refresh_token: None,
        expires_at: claims.expires_at,
        profile_id: claims.subject,
    };

    auth::save_auth(&stored)?;
    info!("Authentication successful — token saved");

    Ok(stored)
}

struct JwtClaims {
    expires_at: DateTime<Utc>,
    subject: Option<uuid::Uuid>,
}

/// Decode JWT payload without verification (just extract exp and sub).
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
    fn decode_jwt_extracts_exp_and_sub() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"EdDSA","typ":"JWT"}"#);
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
    fn decode_jwt_rejects_malformed() {
        assert!(decode_jwt_claims("not.a.valid-jwt").is_err());
        assert!(decode_jwt_claims("only-one-part").is_err());
    }
}
