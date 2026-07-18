//! The token-endpoint exchange. The only outbound HTTP in temper-services.
//!
//! Deliberately not shared with temper-client's copy: sharing it would mean either putting
//! reqwest in the neutral crate (bloating the CLI) or inverting the server->client dependency.
//! The wire TYPE is shared (`temper_auth::TokenResponse`); ~20 lines of form POST are not.

use std::sync::LazyLock;
use std::time::Duration;

use temper_auth::TokenResponse;

use crate::error::{ApiError, ApiResult};

/// One shared client, built once. `reqwest::Client` owns a connection pool and is designed to be
/// reused — a fresh `Client::new()` per call throws that pool away and pays a full TLS handshake
/// every time, which matters most on the refresh path where the call is made while HOLDING a DB
/// row lock (see `slack_grant_vault_service::mint_access_token`). Cloning is cheap (`Arc`).
static HTTP: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// Exchange an authorization code for tokens (RFC 6749 §4.1.3) with PKCE.
///
/// Never logs the code, the verifier, or any token.
pub async fn exchange_code(
    token_url: &str,
    client_id: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> ApiResult<TokenResponse> {
    let resp = HTTP
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("token exchange transport failure: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        // The IdP's body can echo request parameters; log the status only.
        tracing::warn!(%status, "slack link: token exchange rejected by the IdP");
        return Err(ApiError::Unauthorized("token exchange failed".to_string()));
    }

    resp.json::<TokenResponse>()
        .await
        .map_err(|e| ApiError::Internal(format!("token response was not the expected shape: {e}")))
}

/// Spend a refresh token for a fresh access token (RFC 6749 §6), same public PKCE client, no
/// secret. Auth0 rotates the RT on every such call, so the response's `refresh_token` (when
/// present) SUPERSEDES the one spent — the caller must persist it and never replay the old one,
/// or reuse-detection kills the whole grant family.
///
/// Never logs the token, the response, or the client's reply body.
pub async fn refresh_grant(
    token_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> ApiResult<TokenResponse> {
    let resp = HTTP
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ])
        // The vault refreshes under a `SELECT ... FOR UPDATE` row lock, holding a pooled
        // connection across this call — bound it so a hung IdP cannot pin the connection (and the
        // lock) indefinitely.
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("token refresh transport failure: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        // A non-2xx here usually means the grant family is dead (rotated-out RT, revoked at the
        // IdP, or the user un-consented). Log the status only; the body can echo the token.
        tracing::warn!(%status, "slack grant vault: refresh rejected by the IdP");
        return Err(ApiError::Unauthorized("refresh grant failed".to_string()));
    }

    resp.json::<TokenResponse>().await.map_err(|e| {
        ApiError::Internal(format!("refresh response was not the expected shape: {e}"))
    })
}

/// Best-effort revocation of a refresh-token grant at an external IdP.
///
/// Callers MUST treat a failure as non-fatal — see the disconnect service. The
/// token is a body parameter, which is why revocation has to happen *before*
/// the stored ciphertext is deleted.
///
/// Auth0's revocation endpoint returns 200 with an empty body; there is nothing
/// to decode. A public client (no secret) sends only `client_id`.
pub async fn revoke_grant(revoke_url: &str, client_id: &str, refresh_token: &str) -> ApiResult<()> {
    let res = HTTP
        .post(revoke_url)
        .timeout(Duration::from_secs(5))
        .form(&[("client_id", client_id), ("token", refresh_token)])
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("token revoke transport failure: {e}")))?;

    let status = res.status();
    if !status.is_success() {
        // Status only — the body may echo request parameters.
        tracing::warn!(%status, "token revoke returned a non-success status");
        return Err(ApiError::Unauthorized("revoke grant failed".to_string()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The body matchers are the point of this test, not the 200. Auth0's
    /// revocation endpoint identifies the grant *from the token in the body* —
    /// a request that reaches the right URL without carrying `token` revokes
    /// nothing and still returns success. Without these matchers, deleting the
    /// `.form(...)` call entirely would leave this test green.
    #[tokio::test]
    async fn revoke_posts_the_token_and_client_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .and(body_string_contains("token=rt-value"))
            .and(body_string_contains("client_id=test-client"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/oauth/revoke", server.uri());
        revoke_grant(&url, "test-client", "rt-value")
            .await
            .expect("revoke should succeed on 200");
    }

    #[tokio::test]
    async fn revoke_surfaces_a_non_2xx_as_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        let url = format!("{}/oauth/revoke", server.uri());
        let err = revoke_grant(&url, "test-client", "rt-value")
            .await
            .expect_err("a 400 must be an error");
        assert!(matches!(err, ApiError::Unauthorized(_)), "got {err:?}");
    }
}
