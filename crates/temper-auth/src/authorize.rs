//! Authorization-endpoint URL construction.

use crate::AuthError;

/// Inputs to an `/authorize` redirect. A struct rather than seven positional
/// parameters, per the repo's >5-parameter rule.
#[derive(Debug, Clone)]
pub struct AuthorizeParams {
    /// Authorization endpoint (e.g. `https://temperkb.us.auth0.com/authorize`).
    pub authorize_url: String,
    pub client_id: String,
    /// API audience, sent as the `audience` parameter. Omitted entirely when `None`.
    pub audience: Option<String>,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    /// Opaque to this crate. The CLI passes its loopback port; the Slack link flow
    /// passes a DB-backed single-use nonce.
    pub state: String,
    pub code_challenge: String,
}

/// Build the full authorization URL with PKCE parameters.
pub fn build_authorize_url(params: &AuthorizeParams) -> Result<String, AuthError> {
    let scope = params.scopes.join(" ");

    let mut url = url::Url::parse(&params.authorize_url)
        .map_err(|e| AuthError::InvalidAuthorizeUrl(format!("{}: {e}", params.authorize_url)))?;

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &params.client_id)
        .append_pair("redirect_uri", &params.redirect_uri)
        .append_pair("code_challenge", &params.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &params.state)
        .append_pair("scope", &scope);

    if let Some(audience) = &params.audience {
        url.query_pairs_mut().append_pair("audience", audience);
    }

    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> AuthorizeParams {
        AuthorizeParams {
            authorize_url: "https://id.example.com/authorize".to_string(),
            client_id: "cid".to_string(),
            audience: Some("https://api.example.com".to_string()),
            redirect_uri: "https://temperkb.io/api/auth/slack/callback".to_string(),
            scopes: vec!["openid".to_string(), "offline_access".to_string()],
            state: "opaque-nonce".to_string(),
            code_challenge: "chal".to_string(),
        }
    }

    #[test]
    fn builds_url_with_opaque_state_and_s256() {
        let url = build_authorize_url(&params()).unwrap();
        assert!(url.contains("state=opaque-nonce"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("audience=https%3A%2F%2Fapi.example.com"));
        assert!(url.contains("scope=openid+offline_access"));
    }

    #[test]
    fn audience_is_omitted_when_absent() {
        let mut p = params();
        p.audience = None;
        let url = build_authorize_url(&p).unwrap();
        assert!(!url.contains("audience="));
    }

    #[test]
    fn malformed_authorize_url_is_an_error_not_a_panic() {
        let mut p = params();
        p.authorize_url = "not a url".to_string();
        assert!(matches!(
            build_authorize_url(&p),
            Err(AuthError::InvalidAuthorizeUrl(_))
        ));
    }
}
