//! Neon Auth (Better Auth) login flow with local callback server.
//!
//! 1. POST to Neon Auth `/sign-in/social` — get Google OAuth redirect URL + challenge cookie
//! 2. Open browser for Google sign-in
//! 3. Neon Auth redirects to temperkb.io/api/auth-callback which passes the
//!    session verifier back to our localhost server
//! 4. CLI uses the challenge cookie + verifier to exchange for session cookies
//! 5. CLI uses session cookies to fetch JWT from `/auth/token`
//! 6. Persist the token to `~/.config/temper/auth.json`

use chrono::{DateTime, Utc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

use crate::auth::{self, StoredAuth};
use crate::error::{ClientError, Result};

/// Configuration for the OAuth sign-in flow.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// Authorization endpoint URL
    pub authorize_url: String,
    /// Token endpoint URL
    pub token_url: String,
    /// OAuth client ID
    pub client_id: String,
    /// Optional audience parameter (required by Auth0)
    pub audience: Option<String>,
    /// OAuth scopes to request
    pub scopes: Vec<String>,
}

impl OAuthConfig {
    fn neon_auth_base(&self) -> &str {
        &self.authorize_url
    }

    fn provider(&self) -> &str {
        self.scopes.first().map(|s| s.as_str()).unwrap_or("google")
    }
}

#[derive(Debug, serde::Deserialize)]
struct SignInResponse {
    url: Option<String>,
}

// ---------------------------------------------------------------------------
// Login flow
// ---------------------------------------------------------------------------

pub async fn login(config: &OAuthConfig) -> Result<StoredAuth> {
    let neon_auth = config.neon_auth_base();
    let provider = config.provider();

    // Bind to a random port on localhost.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    // Callback goes through temperkb.io which passes the verifier to localhost.
    let callback_url = format!("https://temperkb.io/api/auth-callback?cli_port={port}");

    debug!(port, "Callback server listening");

    // POST to Neon Auth to initiate social sign-in.
    // IMPORTANT: capture the challenge cookie from the response — we need it later.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .cookie_store(true)
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

    // Wait for the callback. The temperkb.io callback page fetches the JWT
    // client-side and redirects to localhost:{port}/callback?verifier=<jwt>
    let verifier_or_jwt = wait_for_verifier(&listener).await?;

    // If it starts with eyJ, it's already a JWT (from the client-side fetch)
    let jwt = if verifier_or_jwt.starts_with("eyJ") {
        debug!("Received JWT directly from callback page");
        verifier_or_jwt
    } else {
        // It's a session verifier — try to exchange it (may not work due to cookie issues)
        debug!("Got session verifier, attempting exchange...");
        let callback_resp = client
            .get(format!(
                "{neon_auth}/callback/{provider}?neon_auth_session_verifier={verifier_or_jwt}"
            ))
            .send()
            .await?;
        debug!(status = %callback_resp.status(), "Verifier exchange");

        let token_resp = client
            .get(format!("{neon_auth}/token"))
            .header("Accept", "application/json")
            .send()
            .await?;

        if !token_resp.status().is_success() {
            let body = token_resp.text().await.unwrap_or_default();
            return Err(ClientError::Other(format!(
                "JWT token request failed: {body}"
            )));
        }

        let data: serde_json::Value = token_resp.json().await?;
        data.get("token")
            .or_else(|| data.get("access_token"))
            .or_else(|| data.get("jwt"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| ClientError::Other(format!("no JWT in response: {data}")))?
            .to_owned()
    };

    let claims = decode_jwt_claims(&jwt)?;

    let stored = StoredAuth {
        provider: provider.to_owned(),
        access_token: jwt.to_owned(),
        refresh_token: None,
        expires_at: claims.expires_at,
        profile_id: claims.subject,
    };

    auth::save_auth(&stored)?;
    info!("Authentication successful — token saved");

    Ok(stored)
}

/// Wait for the callback redirect from temperkb.io with the session verifier.
async fn wait_for_verifier(listener: &TcpListener) -> Result<String> {
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

        if method == "GET" && path.starts_with("/callback") {
            // Parse verifier from query string
            let full_url = url::Url::parse(&format!("http://localhost{path}"))
                .map_err(|e| ClientError::Other(format!("parse error: {e}")))?;

            let verifier = full_url
                .query_pairs()
                .find(|(k, _)| k == "verifier")
                .map(|(_, v)| v.into_owned());

            // Send success response
            let html = "<!DOCTYPE html><html><body>\
                <h2>Completing authentication...</h2>\
                <p>You can close this tab once the CLI confirms success.</p>\
                </body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = stream.write_all(response.as_bytes()).await;

            if let Some(v) = verifier {
                return Ok(v);
            }
            // No verifier in callback — keep waiting (might be favicon request etc.)
        } else {
            // Favicon or other request — ignore
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
