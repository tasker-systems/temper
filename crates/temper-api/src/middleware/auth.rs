use axum::body::Body;
use axum::extract::{FromRequestParts, State};
use axum::http::{request::Parts, Request};
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{decode, TokenData};
use serde::Deserialize;
use std::future::Future;

use temper_core::types::{AuthClaims, AuthenticatedProfile};

use temper_services::error::ApiError;
use temper_services::services::profile_service;
use temper_services::state::AppState;

/// Internal JWT claim structure for deserialization.
#[derive(Debug, Deserialize)]
struct JwtClaims {
    sub: String,
    email: Option<String>,
    email_verified: Option<bool>,
    exp: i64,
    iat: i64,
}

/// Newtype carrying the value of the `X-Temper-Device-Id` request header.
#[derive(Debug, Clone)]
pub struct DeviceId(pub String);

/// Local wrapper around [`AuthenticatedProfile`] that implements axum's
/// [`FromRequestParts`] extractor. Route handlers use `AuthUser` and
/// access the inner value via `.0`.
#[derive(Debug, Clone)]
pub struct AuthUser(pub AuthenticatedProfile);

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = parts
            .extensions
            .get::<AuthenticatedProfile>()
            .cloned()
            .map(AuthUser)
            .ok_or(ApiError::Unauthorized(
                "Authentication required".to_string(),
            ));
        std::future::ready(result)
    }
}

/// Axum middleware that verifies a Bearer JWT, resolves or auto-provisions the
/// corresponding profile, and injects [`AuthenticatedProfile`] into request
/// extensions for downstream handlers.
pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    // 1. Extract "Authorization: Bearer <token>"
    let token = extract_bearer_token(&request)?;

    // 2. Get the current decoding key from the JWKS store (cached).
    let decoding_key = state.jwks_store.get_decoding_key().await.map_err(|e| {
        tracing::error!("JWKS key retrieval failed: {e}");
        ApiError::Unauthorized("Authentication service unavailable".to_string())
    })?;

    // 3. Decode and verify the JWT.
    let issuer = &state.config.auth_issuer;
    let audience = state.config.auth_audience.as_deref();
    let validation = state.jwks_store.validation(issuer, audience);

    let token_data: TokenData<JwtClaims> =
        decode(&token, &decoding_key, &validation).map_err(|e| {
            tracing::debug!("JWT verification failed: {e}");
            ApiError::Unauthorized("Invalid or expired token".to_string())
        })?;

    // 4. Resolve email — present in token claims (custom Auth0 Action),
    //    cached in kb_profile_auth_links from a prior login, or fetched
    //    from the OIDC /userinfo endpoint as a last resort.
    let (email, email_verified) =
        resolve_email_from_claims(&state, &token_data.claims, &token).await?;

    let claims = AuthClaims {
        provider: state.config.auth_provider_name.clone(),
        external_user_id: token_data.claims.sub,
        email,
        email_verified,
        exp: token_data.claims.exp,
        iat: token_data.claims.iat,
    };

    // 5. Resolve (or auto-provision) the profile.
    let profile = profile_service::resolve_from_claims(&state.pool, &claims).await?;

    tracing::Span::current().record("profile_id", tracing::field::display(profile.id));

    // 6. Optionally capture X-Temper-Device-Id.
    let device_id = request
        .headers()
        .get("X-Temper-Device-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| DeviceId(s.to_string()));

    if let Some(id) = device_id {
        request.extensions_mut().insert(id);
    }

    // 7. Inject AuthenticatedProfile into extensions.
    request
        .extensions_mut()
        .insert(AuthenticatedProfile { profile, claims });

    // 8. Continue.
    Ok(next.run(request).await)
}

/// Resolve the caller's email (and its verified flag) from JWT claims.
///
/// Tries three sources in order: the email claim embedded in the token (custom
/// Auth0 Action), a previously cached email in `kb_profile_auth_links`, then the
/// OIDC `/userinfo` endpoint as a last resort. Failure to resolve via userinfo is
/// surfaced as [`ApiError::Unauthorized`].
async fn resolve_email_from_claims(
    state: &AppState,
    claims: &JwtClaims,
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

/// Extract a Bearer token from the `Authorization` header.
fn extract_bearer_token(request: &Request<Body>) -> Result<String, ApiError> {
    let header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .ok_or_else(|| ApiError::Unauthorized("Missing Authorization header".to_string()))?;

    let value = header
        .to_str()
        .map_err(|_| ApiError::Unauthorized("Invalid Authorization header encoding".to_string()))?;

    let token = value.strip_prefix("Bearer ").ok_or_else(|| {
        ApiError::Unauthorized("Authorization header must use Bearer scheme".to_string())
    })?;

    Ok(token.to_string())
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
