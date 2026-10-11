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

/// What spending a refresh token produced.
///
/// Two arms rather than `ApiResult<TokenResponse>` alone, because one failure is not like the
/// others: an IdP answering `invalid_grant` is telling us the grant family is DEAD — the RT expired
/// (Auth0's inactivity/absolute lifetime, the AS's `AS_REFRESH_TTL_SECONDS`), was rotated out, was
/// revoked, or tripped reuse-detection. No retry of this call can ever succeed, and the only remedy
/// is a fresh consent. Folding that into the same `Err` as a 5xx or a transport failure is what made
/// a Slack user whose grant had simply aged out read "try again in a moment" forever.
///
/// Every OTHER non-2xx stays an `Err`: `invalid_client`, a 5xx, a rate limit are deployment or
/// transient facts, not facts about this user's grant, and must not be treated as license to
/// discard it.
#[derive(Debug)]
pub enum RefreshOutcome {
    /// The IdP issued tokens. Auth0 rotates, so `refresh_token` (when present) supersedes the RT
    /// that was spent.
    Refreshed(TokenResponse),
    /// The IdP refused the grant itself (`invalid_grant`, RFC 6749 §5.2). Permanent.
    GrantRejected,
}

/// The only part of an RFC 6749 §5.2 error body this module reads. `error_description` is
/// deliberately not modelled: it is free text the IdP may echo request material into, and nothing
/// here branches on it.
#[derive(serde::Deserialize)]
struct OAuthErrorBody {
    error: String,
}

/// Spend a refresh token for a fresh access token (RFC 6749 §6), same public PKCE client, no
/// secret. Auth0 rotates the RT on every such call, so the response's `refresh_token` (when
/// present) SUPERSEDES the one spent — the caller must persist it and never replay the old one,
/// or reuse-detection kills the whole grant family.
///
/// An `invalid_grant` refusal is [`RefreshOutcome::GrantRejected`], not an error — see that type.
/// It is recognised by the body's `error` code and a 4xx status together, never by status alone:
/// Auth0 answers `invalid_grant` with 403 and the AS with 400, while a 401/403 can equally mean
/// `invalid_client`, which is a broken deployment rather than a dead grant.
///
/// Never logs the token, the response, or the client's reply body beyond its `error` code (a fixed
/// RFC 6749 vocabulary, not request material).
pub async fn refresh_grant(
    token_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> ApiResult<RefreshOutcome> {
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

    let status = resp.status();
    if !status.is_success() {
        // Read only the `error` code; an unparseable body is simply "not invalid_grant".
        let error_code = if status.is_client_error() {
            resp.json::<OAuthErrorBody>().await.ok().map(|b| b.error)
        } else {
            None
        };
        if error_code.as_deref() == Some("invalid_grant") {
            tracing::warn!(
                %status,
                "slack grant vault: the IdP rejected the refresh grant as invalid_grant; the grant \
                 family is dead and only a re-link can replace it"
            );
            return Ok(RefreshOutcome::GrantRejected);
        }
        tracing::warn!(
            %status,
            error = error_code.as_deref().unwrap_or("unknown"),
            "slack grant vault: refresh rejected by the IdP"
        );
        return Err(ApiError::Unauthorized("refresh grant failed".to_string()));
    }

    resp.json::<TokenResponse>()
        .await
        .map(RefreshOutcome::Refreshed)
        .map_err(|e| {
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

    async fn refresh_against(status: u16, body: serde_json::Value) -> ApiResult<RefreshOutcome> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        let url = format!("{}/oauth/token", server.uri());
        refresh_grant(&url, "test-client", "rt-value").await
    }

    /// Auth0's shape for an expired / rotated-out / revoked RT: 403 + `invalid_grant`. This is the
    /// production failure that read as "try again" forever before the refusal was typed.
    #[tokio::test]
    async fn an_auth0_invalid_grant_is_a_rejected_grant_not_an_error() {
        let outcome = refresh_against(
            403,
            serde_json::json!({"error": "invalid_grant", "error_description": "Unknown or invalid refresh token."}),
        )
        .await
        .expect("invalid_grant is an outcome, not an error");
        assert!(
            matches!(outcome, RefreshOutcome::GrantRejected),
            "got {outcome:?}"
        );
    }

    /// The temper AS answers the same refusal with 400 (`packages/temper-cloud/src/oauth/endpoints.ts`).
    #[tokio::test]
    async fn an_as_invalid_grant_is_a_rejected_grant_not_an_error() {
        let outcome = refresh_against(400, serde_json::json!({"error": "invalid_grant"}))
            .await
            .expect("invalid_grant is an outcome, not an error");
        assert!(
            matches!(outcome, RefreshOutcome::GrantRejected),
            "got {outcome:?}"
        );
    }

    /// A 401/403 is NOT enough on its own: `invalid_client` is a broken deployment, and treating it
    /// as a dead grant would have the caller discard every user's working grant.
    #[tokio::test]
    async fn invalid_client_stays_an_error() {
        let err = refresh_against(401, serde_json::json!({"error": "invalid_client"}))
            .await
            .expect_err("invalid_client must not read as a dead grant");
        assert!(matches!(err, ApiError::Unauthorized(_)), "got {err:?}");
    }

    /// A server error carrying the same code is still not a grant verdict — only a 4xx is.
    #[tokio::test]
    async fn a_5xx_stays_an_error_whatever_its_body_says() {
        let err = refresh_against(500, serde_json::json!({"error": "invalid_grant"}))
            .await
            .expect_err("a 5xx is transient, never a dead grant");
        assert!(matches!(err, ApiError::Unauthorized(_)), "got {err:?}");
    }
}
