//! The human email ladder — the single place either surface resolves a human
//! token's email.
//!
//! Lifted out of `temper-api`'s auth middleware, unchanged. It lived on one surface
//! because only one surface had it: temper-mcp set `email: String::new()` and
//! auto-provisioned a profile with an empty email. Two surfaces, two answers to
//! "who is this human", from the same token — the same class of drift the closed
//! [`super::Principal`] sum closed for *classification* (see `normalize.rs`), one
//! level down at *construction*.
//!
//! The ladder is deliberately concrete rather than a trait: there is exactly one
//! implementation and no policy to vary. A surface that wanted a different email
//! answer would be re-introducing the drift.
//!
//! Three rungs, in order — each cheaper and more trustworthy than the next:
//! 1. the `email` claim on the token (our Auth0 Action puts it there),
//! 2. the email cached on `kb_profile_auth_links` from an earlier sign-in,
//! 3. the OIDC `/userinfo` endpoint, discovered once per process.
//!
//! Falling off the bottom is [`ApiError::Unauthorized`]: a human we cannot name is
//! a human we will not provision.

use serde::Deserialize;

use crate::error::ApiError;
use crate::state::AppState;

use super::RawJwtClaims;

/// Resolve the caller's email (and its verified flag) from JWT claims.
///
/// Tries three sources in order: the email claim embedded in the token (custom
/// Auth0 Action), a previously cached email in `kb_profile_auth_links`, then the
/// OIDC `/userinfo` endpoint as a last resort. Failure to resolve via userinfo is
/// surfaced as [`ApiError::Unauthorized`].
pub(super) async fn resolve_email_from_claims(
    state: &AppState,
    claims: &RawJwtClaims,
    token: &str,
) -> Result<(String, Option<bool>), ApiError> {
    if let Some(email) = &claims.email {
        return Ok((email.clone(), claims.email_verified));
    }

    // Check the DB for a previously resolved email before hitting userinfo.
    let cached =
        lookup_cached_email(&state.pool, &state.config.auth_provider_name, &claims.sub).await;

    match cached {
        Some((email, _)) => {
            tracing::debug!("resolved email from cached auth link");
            Ok((email, Some(true)))
        }
        None => {
            let endpoint = state
                .userinfo_endpoint
                .get_or_try_init(|| discover_userinfo_endpoint(&state.config.auth_issuer))
                .await
                .map_err(|e| {
                    tracing::warn!("OIDC discovery failed: {e}");
                    ApiError::Unauthorized(
                        "Token missing email claim and userinfo lookup failed".to_string(),
                    )
                })?;
            fetch_email_from_userinfo(endpoint, token)
                .await
                .map_err(|e| {
                    tracing::warn!("Failed to fetch email from userinfo: {e}");
                    ApiError::Unauthorized(
                        "Token missing email claim and userinfo lookup failed".to_string(),
                    )
                })
        }
    }
}

/// Look up the email for a known auth link in the database.
///
/// Returns `Some((email, email_verified_placeholder))` if the user has logged
/// in before and we cached their email in `kb_profile_auth_links`.
async fn lookup_cached_email(
    pool: &sqlx::PgPool,
    provider: &str,
    external_user_id: &str,
) -> Option<(String, Option<bool>)> {
    let email = sqlx::query_scalar!(
        "SELECT email FROM kb_profile_auth_links WHERE auth_provider = $1 AND auth_provider_user_id = $2",
        provider,
        external_user_id,
    )
    .fetch_optional(pool)
    .await
    .ok()?
    .flatten();

    email.map(|e| (e, Some(true)))
}

/// Subset of the OIDC discovery document (`/.well-known/openid-configuration`).
#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    userinfo_endpoint: Option<String>,
}

/// Parse the `userinfo_endpoint` out of an OIDC discovery document body.
fn parse_userinfo_endpoint(body: &str) -> Result<String, String> {
    let doc: OidcDiscovery =
        serde_json::from_str(body).map_err(|e| format!("discovery parse error: {e}"))?;
    doc.userinfo_endpoint
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "discovery document missing userinfo_endpoint".to_string())
}

/// Resolve the OIDC userinfo endpoint for `issuer` via discovery.
async fn discover_userinfo_endpoint(issuer: &str) -> Result<String, String> {
    let base = issuer.trim_end_matches('/');
    let url = format!("{base}/.well-known/openid-configuration");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("http client build failed: {e}"))?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("discovery request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("discovery returned status {}", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("discovery read error: {e}"))?;
    parse_userinfo_endpoint(&body)
}

/// OIDC userinfo response (subset of fields we need).
#[derive(Debug, Deserialize)]
struct UserinfoResponse {
    email: Option<String>,
    email_verified: Option<bool>,
}

/// Fetch the user's email from a resolved OIDC `/userinfo` endpoint.
async fn fetch_email_from_userinfo(
    userinfo_url: &str,
    access_token: &str,
) -> Result<(String, Option<bool>), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("http client build failed: {e}"))?;
    let resp = client
        .get(userinfo_url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("userinfo request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("userinfo returned status {}", resp.status()));
    }

    let info: UserinfoResponse = resp
        .json()
        .await
        .map_err(|e| format!("userinfo parse error: {e}"))?;

    let email = info
        .email
        .ok_or_else(|| "userinfo response missing email field".to_string())?;

    Ok((email, info.email_verified))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_userinfo_endpoint_auth0_shape() {
        let body = r#"{"issuer":"https://t.auth0.com/","userinfo_endpoint":"https://t.auth0.com/userinfo"}"#;
        assert_eq!(
            parse_userinfo_endpoint(body).unwrap(),
            "https://t.auth0.com/userinfo"
        );
    }

    #[test]
    fn parse_userinfo_endpoint_okta_shape() {
        let body = r#"{"issuer":"https://org.okta.com/oauth2/aus1","userinfo_endpoint":"https://org.okta.com/oauth2/aus1/v1/userinfo"}"#;
        assert_eq!(
            parse_userinfo_endpoint(body).unwrap(),
            "https://org.okta.com/oauth2/aus1/v1/userinfo"
        );
    }

    #[test]
    fn parse_userinfo_endpoint_missing_field_errors() {
        let body = r#"{"issuer":"https://x"}"#;
        assert!(parse_userinfo_endpoint(body).is_err());
    }
}
