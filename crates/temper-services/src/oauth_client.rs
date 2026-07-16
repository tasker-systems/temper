//! The token-endpoint exchange. The only outbound HTTP in temper-services.
//!
//! Deliberately not shared with temper-client's copy: sharing it would mean either putting
//! reqwest in the neutral crate (bloating the CLI) or inverting the server->client dependency.
//! The wire TYPE is shared (`temper_auth::TokenResponse`); ~20 lines of form POST are not.

use temper_auth::TokenResponse;

use crate::error::{ApiError, ApiResult};

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
    let client = reqwest::Client::new();
    let resp = client
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
