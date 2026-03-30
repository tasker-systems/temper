# Auth0 Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Neon Auth with Auth0 as the sole authentication provider, enabling standard OAuth2 PKCE for CLI login with provider-agnostic configuration.

**Architecture:** Update JWKS middleware (Rust + TypeScript) to accept RS256, rewrite CLI login flow to use standard OAuth2 Authorization Code + PKCE directly against Auth0, ship compiled-in defaults for temperkb.io + Auth0, remove Neon Auth relay endpoints.

**Tech Stack:** Rust (jsonwebtoken, reqwest, sha2, base64, url), TypeScript (jose), Auth0 (`temperkb.us.auth0.com`)

**Spec:** `docs/superpowers/specs/2026-03-30-auth0-integration-design.md`

---

### Task 1: Update JWKS Key Store to Accept RSA Keys

**Files:**
- Modify: `crates/temper-api/src/state.rs`

- [ ] **Step 1: Write failing test for RSA key acceptance**

Add to the `mod tests` block in `state.rs`:

```rust
#[test]
fn is_rsa_key_accepted() {
    use jsonwebtoken::jwk::{AlgorithmParameters, RSAKeyParameters, RSAKeyType};
    let params = AlgorithmParameters::RSA(RSAKeyParameters {
        key_type: RSAKeyType::RSA,
        n: "test".to_string(),
        e: "test".to_string(),
    });
    assert!(is_supported_key(&params));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p temper-api is_rsa_key_accepted`
Expected: FAIL — `is_supported_key` does not exist yet

- [ ] **Step 3: Rename `is_ed25519_okp` to `is_supported_key` and accept RSA**

Replace the `is_ed25519_okp` function and its usages:

```rust
/// Check if a JWK is a key type we support for JWT verification.
/// Accepts RSA keys (for RS256) and OKP/Ed25519 keys (for EdDSA).
fn is_supported_key(params: &AlgorithmParameters) -> bool {
    match params {
        AlgorithmParameters::RSA(_) => true,
        AlgorithmParameters::OctetKeyPair(p) if p.curve == EllipticCurve::Ed25519 => true,
        _ => false,
    }
}
```

Update the `refresh` method's filter from `is_ed25519_okp` to `is_supported_key`:

```rust
let decoding_key = jwks
    .keys
    .iter()
    .filter(|jwk| is_supported_key(&jwk.algorithm))
    .find_map(|jwk| DecodingKey::from_jwk(jwk).ok())
    .ok_or_else(|| "No supported key (RSA or Ed25519) found in JWKS response".to_string())?;
```

- [ ] **Step 4: Update the `validation` method to accept both RS256 and EdDSA**

```rust
pub fn validation(&self, issuer: &str, audience: Option<&str>) -> Validation {
    let mut v = Validation::new(Algorithm::RS256);
    v.algorithms = vec![Algorithm::RS256, Algorithm::EdDSA];
    v.set_issuer(&[issuer]);
    if let Some(aud) = audience {
        v.set_audience(&[aud]);
    } else {
        v.validate_aud = false;
    }
    v
}
```

- [ ] **Step 5: Update existing tests that assert EdDSA-only behavior**

In the test `validation_with_audience_enables_aud_check`, change:

```rust
assert!(v.algorithms.contains(&Algorithm::RS256));
assert!(v.algorithms.contains(&Algorithm::EdDSA));
```

Rename `is_ed25519_okp_accepts_valid_key` to `is_supported_key_accepts_ed25519`:
```rust
#[test]
fn is_supported_key_accepts_ed25519() {
    use jsonwebtoken::jwk::{
        AlgorithmParameters, EllipticCurve, OctetKeyPairParameters, OctetKeyPairType,
    };
    let params = AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
        key_type: OctetKeyPairType::OctetKeyPair,
        curve: EllipticCurve::Ed25519,
        x: "test".to_string(),
    });
    assert!(is_supported_key(&params));
}
```

Rename `is_ed25519_okp_rejects_rsa` to `is_supported_key_accepts_rsa` and flip the assertion:
```rust
#[test]
fn is_supported_key_accepts_rsa() {
    use jsonwebtoken::jwk::{AlgorithmParameters, RSAKeyParameters, RSAKeyType};
    let params = AlgorithmParameters::RSA(RSAKeyParameters {
        key_type: RSAKeyType::RSA,
        n: "test".to_string(),
        e: "test".to_string(),
    });
    assert!(is_supported_key(&params));
}
```

Rename `is_ed25519_okp_rejects_wrong_curve` to `is_supported_key_rejects_wrong_curve` (keep same body, just rename the function reference from `is_ed25519_okp` to `is_supported_key`).

- [ ] **Step 6: Run all tests**

Run: `cargo test -p temper-api`
Expected: All tests pass

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/state.rs
git commit -m "feat: accept RS256 and EdDSA keys in JWKS key store"
```

---

### Task 2: Update TypeScript JWT Verification for RS256

**Files:**
- Modify: `packages/temper-cloud/src/auth.ts`

- [ ] **Step 1: Add RS256 to allowed algorithms**

In `verifyToken`, change the algorithms array:

```typescript
const opts: jose.JWTVerifyOptions = { issuer, algorithms: ["RS256", "EdDSA"] };
```

- [ ] **Step 2: Run TypeScript type check**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && npx tsc --noEmit --project tsconfig.api.json`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add packages/temper-cloud/src/auth.ts
git commit -m "feat: accept RS256 JWTs in TypeScript verification"
```

---

### Task 3: Add `audience` Field to Client Config and Update Defaults

**Files:**
- Modify: `crates/temper-client/src/config.rs`

- [ ] **Step 1: Write failing test for audience field and new defaults**

Add to `mod tests` in `config.rs`:

```rust
#[test]
fn default_provider_is_auth0() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    // No file — defaults should provide auth0 config
    let config = load_cloud_config_from(&path).unwrap();
    assert_eq!(config.auth.provider, "auth0");
    let provider = config.auth.providers.get("auth0").unwrap();
    assert_eq!(provider.authorize_url, "https://temperkb.us.auth0.com/authorize");
    assert_eq!(provider.token_url, "https://temperkb.us.auth0.com/oauth/token");
    assert_eq!(provider.client_id, "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF");
    assert_eq!(provider.audience, Some("https://temperkb.io/api".to_string()));
    assert!(provider.scopes.contains(&"offline_access".to_string()));
}

#[test]
fn config_file_overrides_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    let toml = r#"
[auth]
provider = "keycloak"

[auth.providers.keycloak]
authorize_url = "https://sso.example.com/auth"
token_url     = "https://sso.example.com/token"
client_id     = "custom-client"
audience      = "custom-api"
scopes        = ["openid", "profile"]
"#;
    fs::write(&path, toml).unwrap();
    let config = load_cloud_config_from(&path).unwrap();
    assert_eq!(config.auth.provider, "keycloak");
    let p = config.auth.providers.get("keycloak").unwrap();
    assert_eq!(p.audience, Some("custom-api".to_string()));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p temper-client default_provider_is_auth0`
Expected: FAIL — default provider is still `neon_auth`, no compiled-in providers

- [ ] **Step 3: Add `audience` field to `ProviderConfig`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    #[serde(default)]
    pub audience: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
}
```

- [ ] **Step 4: Update defaults to Auth0**

Replace `default_provider`:

```rust
fn default_provider() -> String {
    "auth0".into()
}
```

Add a function that returns the compiled-in Auth0 provider config:

```rust
fn default_providers() -> HashMap<String, ProviderConfig> {
    let mut map = HashMap::new();
    map.insert(
        "auth0".to_string(),
        ProviderConfig {
            authorize_url: "https://temperkb.us.auth0.com/authorize".to_string(),
            token_url: "https://temperkb.us.auth0.com/oauth/token".to_string(),
            client_id: "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF".to_string(),
            audience: Some("https://temperkb.io/api".to_string()),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
            ],
        },
    );
    map
}
```

Update `AuthConfig::default`:

```rust
impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            providers: default_providers(),
        }
    }
}
```

- [ ] **Step 5: Update `oauth_config` to pass `audience` through to `OAuthConfig`**

The `OAuthConfig` struct will be updated in Task 4 to include `audience`. For now, update the mapping in `oauth_config()`:

```rust
pub fn oauth_config(config: &CloudConfig) -> crate::error::Result<crate::login::OAuthConfig> {
    let provider = config
        .auth
        .providers
        .get(&config.auth.provider)
        .ok_or_else(|| {
            crate::error::ClientError::Other(format!(
                "auth provider '{}' not found in config",
                config.auth.provider
            ))
        })?;
    Ok(crate::login::OAuthConfig {
        authorize_url: provider.authorize_url.clone(),
        token_url: provider.token_url.clone(),
        client_id: provider.client_id.clone(),
        audience: provider.audience.clone(),
        scopes: provider.scopes.clone(),
    })
}
```

- [ ] **Step 6: Remove debug `eprintln` statements from `build_client`**

Replace the `build_client` function body:

```rust
pub fn build_client() -> crate::error::Result<crate::TemperClient> {
    let config = load_cloud_config()?;
    let url = api_url(&config);
    let device_id = load_device_id();
    let mut client = crate::TemperClient::new(&url, device_id);
    match oauth_config(&config) {
        Ok(oauth) => {
            client = client.with_oauth(oauth);
        }
        Err(e) => {
            tracing::debug!("OAuth config not available: {e}");
        }
    }
    Ok(client)
}
```

- [ ] **Step 7: Update existing tests that reference `neon_auth`**

In `returns_defaults_when_file_absent`, change:
```rust
assert_eq!(config.auth.provider, "auth0");
```

In `oauth_config_missing_provider_returns_error`, the test needs a config with no `auth0` provider. Create a config explicitly:
```rust
#[test]
fn oauth_config_missing_provider_returns_error() {
    let config = CloudConfig {
        auth: AuthConfig {
            provider: "nonexistent".to_string(),
            providers: HashMap::new(),
        },
        cloud: CloudSection::default(),
    };
    let err = oauth_config(&config).unwrap_err();
    assert!(err.to_string().contains("nonexistent"));
}
```

- [ ] **Step 8: Run all tests**

Run: `cargo test -p temper-client`
Expected: All tests pass

- [ ] **Step 9: Commit**

```bash
git add crates/temper-client/src/config.rs
git commit -m "feat: Auth0 defaults and audience field in client config"
```

---

### Task 4: Rewrite CLI Login Flow for Standard OAuth2 PKCE

**Files:**
- Modify: `crates/temper-client/src/login.rs`

This is the core change. The current flow POSTs to Neon Auth's `/sign-in/social` and uses a complex cookie-based exchange. The new flow builds a standard `/authorize` URL with PKCE parameters and exchanges the authorization code at `/oauth/token`.

- [ ] **Step 1: Write failing test for PKCE challenge generation**

Add to `mod tests` in `login.rs`:

```rust
#[test]
fn pkce_challenge_is_valid_s256() {
    let (verifier, challenge) = generate_pkce_pair();
    // Verifier: 43-128 characters, URL-safe
    assert!(verifier.len() >= 43 && verifier.len() <= 128);
    // Challenge: base64url-encoded SHA256 of verifier
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
        scopes: vec!["openid".to_string(), "profile".to_string(), "email".to_string()],
    };
    let (verifier, challenge) = generate_pkce_pair();
    let url = build_authorize_url(&config, 12345, &challenge);
    assert!(url.contains("response_type=code"));
    assert!(url.contains("client_id=test-client-id"));
    assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A12345%2Fcallback"));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains(&format!("code_challenge={challenge}")));
    assert!(url.contains("audience=https%3A%2F%2Ftemperkb.io%2Fapi"));
    assert!(url.contains("scope=openid+profile+email"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p temper-client pkce_challenge_is_valid`
Expected: FAIL — `generate_pkce_pair` and `build_authorize_url` do not exist

- [ ] **Step 3: Add sha2 dependency to temper-client**

In `crates/temper-client/Cargo.toml`, add under `[dependencies]`:

```toml
sha2 = "0.10"
```

Check if sha2 is already in the workspace `Cargo.toml` — if so, use `sha2.workspace = true` instead.

- [ ] **Step 4: Rewrite `login.rs`**

Replace the entire file contents:

```rust
//! OAuth2 Authorization Code + PKCE login flow with local callback server.
//!
//! 1. Generate PKCE code_verifier and code_challenge
//! 2. Open browser to provider's /authorize endpoint
//! 3. Provider redirects to http://localhost:{port}/callback?code=...
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

    // 32 random bytes → 43-character base64url string
    let random_bytes: [u8; 32] = rand::random();
    let verifier = URL_SAFE_NO_PAD.encode(random_bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    (verifier, challenge)
}

/// Build the full authorization URL with PKCE parameters.
pub fn build_authorize_url(config: &OAuthConfig, port: u16, code_challenge: &str) -> String {
    let redirect_uri = format!("http://localhost:{port}/callback");
    let scope = config.scopes.join(" ");

    let mut url = url::Url::parse(&config.authorize_url)
        .expect("authorize_url must be a valid URL");

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
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
        return Err(ClientError::Other(format!(
            "Token exchange failed: {body}"
        )));
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
    let redirect_uri = format!("http://localhost:{port}/callback");

    debug!(port, "Callback server listening");

    // Build authorization URL and open browser.
    let auth_url = build_authorize_url(config, port, &code_challenge);

    info!("Opening browser for authentication...");
    open::that(&auth_url)
        .map_err(|e| ClientError::Other(format!("failed to open browser: {e}")))?;

    // Wait for the callback with the authorization code.
    let code = wait_for_code(&listener).await?;

    debug!("Authorization code received, exchanging for tokens...");

    // Exchange code for tokens.
    let tokens = exchange_code(config, &code, &code_verifier, &redirect_uri).await?;

    // Decode claims from the access token (or id_token as fallback).
    let token_to_decode = &tokens.access_token;
    let claims = decode_jwt_claims(token_to_decode)?;

    let expires_at = if let Some(exp) = tokens.expires_in {
        Utc::now() + chrono::Duration::seconds(exp as i64)
    } else {
        claims.expires_at
    };

    let stored = StoredAuth {
        provider: "auth0".to_owned(),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at,
        profile_id: claims.subject,
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

        if method == "GET" && path.starts_with("/callback") {
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
    }

    #[test]
    fn authorize_url_without_audience() {
        let config = OAuthConfig {
            authorize_url: "https://example.com/authorize".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "test".to_string(),
            audience: None,
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
        let payload = URL_SAFE_NO_PAD
            .encode(r#"{"sub":"google-oauth2|123456789","exp":1711800000}"#);
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
```

- [ ] **Step 5: Check for `rand` dependency**

The `generate_pkce_pair` function uses `rand::random()`. Check if `rand` is already a dependency of temper-client. If not, add it to `crates/temper-client/Cargo.toml`:

```toml
rand = "0.8"
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p temper-client`
Expected: All tests pass

- [ ] **Step 7: Run clippy**

Run: `cargo clippy -p temper-client --all-features`
Expected: No warnings

- [ ] **Step 8: Commit**

```bash
git add crates/temper-client/src/login.rs crates/temper-client/Cargo.toml
git commit -m "feat: rewrite CLI login for standard OAuth2 PKCE flow"
```

---

### Task 5: Remove Neon Auth Vercel Endpoints

**Files:**
- Delete: `api/auth-login.ts`
- Delete: `api/auth-callback.ts`

- [ ] **Step 1: Delete the files**

```bash
rm api/auth-login.ts api/auth-callback.ts
```

- [ ] **Step 2: Check for any imports or references to the deleted files**

Run: `grep -r "auth-login\|auth-callback" --include="*.ts" --include="*.json" --include="*.toml" .`

If `vercel.json` or any route config references these endpoints, remove those references.

- [ ] **Step 3: Run TypeScript type check**

Run: `npx tsc --noEmit && npx tsc --noEmit --project tsconfig.api.json`
Expected: No errors (the deleted files should not be imported anywhere)

- [ ] **Step 4: Commit**

```bash
git add -A api/auth-login.ts api/auth-callback.ts
git commit -m "chore: remove Neon Auth relay endpoints"
```

---

### Task 6: Update Vercel Environment Variables

**Files:** None (Vercel dashboard or CLI)

- [ ] **Step 1: Set new Auth0 environment variables**

```bash
vercel env add JWKS_URL production <<< "https://temperkb.us.auth0.com/.well-known/jwks.json"
vercel env add AUTH_ISSUER production <<< "https://temperkb.us.auth0.com/"
vercel env add AUTH_AUDIENCE production <<< "https://temperkb.io/api"
vercel env add AUTH_PROVIDER_NAME production <<< "auth0"
```

Also set for preview environment:
```bash
vercel env add JWKS_URL preview <<< "https://temperkb.us.auth0.com/.well-known/jwks.json"
vercel env add AUTH_ISSUER preview <<< "https://temperkb.us.auth0.com/"
vercel env add AUTH_AUDIENCE preview <<< "https://temperkb.io/api"
vercel env add AUTH_PROVIDER_NAME preview <<< "auth0"
```

- [ ] **Step 2: Remove old Neon Auth variables**

```bash
vercel env rm NEON_AUTH_URL production
vercel env rm NEON_AUTH_URL preview
```

- [ ] **Step 3: Deploy to verify**

```bash
vercel --prod
```

- [ ] **Step 4: Verify the JWKS endpoint is reachable**

```bash
curl -s https://temperkb.us.auth0.com/.well-known/jwks.json | jq '.keys[0].kty'
```

Expected: `"RSA"`

---

### Task 7: Update Local Config and End-to-End Test

**Files:**
- Modify: `~/.config/temper/config.toml`

- [ ] **Step 1: Update local config.toml**

Write or update `~/.config/temper/config.toml`:

```toml
[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]

[cloud]
api_url = "https://temperkb.io"
```

Note: With the compiled-in defaults from Task 3, a fresh install won't need this file at all. This step is for the developer machine that already has a config.toml with old Neon Auth settings.

- [ ] **Step 2: Build and install the updated CLI**

```bash
cargo install --path crates/temper-cli
```

- [ ] **Step 3: Test `temper auth login`**

```bash
temper auth login
```

Expected: Browser opens to Auth0 Universal Login, Google sign-in completes, callback received, token stored. Terminal shows JSON with `authenticated: true`.

- [ ] **Step 4: Test `temper auth status`**

```bash
temper auth status
```

Expected: JSON showing `provider: "auth0"`, valid `expires_at`, and `authenticated: true`.

- [ ] **Step 5: Test `temper auth logout`**

```bash
temper auth logout
```

Expected: `{"status": "logged_out"}`

- [ ] **Step 6: Test API call with Auth0 token**

```bash
temper auth login
# Then make an API call that requires auth, e.g.:
curl -H "Authorization: Bearer $(cat ~/.config/temper/auth.json | jq -r .access_token)" https://temperkb.io/api/profile
```

Expected: Profile JSON returned (or auto-provisioned on first call)

- [ ] **Step 7: Commit config and any fixes**

```bash
git add -A
git commit -m "chore: update config for Auth0 and verify end-to-end"
```

---

### Task 8: Update Default Provider Name in temper-api Config

**Files:**
- Modify: `crates/temper-api/src/config.rs`

- [ ] **Step 1: Update default provider name**

Change the default value:

```rust
let auth_provider_name =
    env::var("AUTH_PROVIDER_NAME").unwrap_or_else(|_| "auth0".to_string());
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p temper-api`
Expected: All pass

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/src/config.rs
git commit -m "chore: default AUTH_PROVIDER_NAME to auth0"
```
