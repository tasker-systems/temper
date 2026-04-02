use axum::body::Body;
use axum::extract::{FromRequestParts, State};
use axum::http::{request::Parts, Request};
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{decode, TokenData};
use serde::Deserialize;
use std::future::Future;

use temper_core::types::{AuthClaims, AuthenticatedProfile};

use crate::error::ApiError;
use crate::services::profile_service;
use crate::state::AppState;

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
    let (email, email_verified) = match token_data.claims.email {
        Some(email) => (email, token_data.claims.email_verified),
        None => {
            // Check the DB for a previously resolved email before hitting userinfo.
            let cached = lookup_cached_email(
                &state.pool,
                &state.config.auth_provider_name,
                &token_data.claims.sub,
            )
            .await;

            match cached {
                Some((email, _)) => {
                    tracing::debug!("resolved email from cached auth link");
                    (email, Some(true))
                }
                None => fetch_email_from_userinfo(&state.config.auth_issuer, &token)
                    .await
                    .map_err(|e| {
                        tracing::warn!("Failed to fetch email from userinfo: {e}");
                        ApiError::Unauthorized(
                            "Token missing email claim and userinfo lookup failed".to_string(),
                        )
                    })?,
            }
        }
    };

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

/// Look up the email for a known auth link in the database.
///
/// Returns `Some((email, email_verified_placeholder))` if the user has logged
/// in before and we cached their email in `kb_profile_auth_links`.
async fn lookup_cached_email(
    pool: &sqlx::PgPool,
    provider: &str,
    external_user_id: &str,
) -> Option<(String, Option<bool>)> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT email FROM kb_profile_auth_links WHERE auth_provider = $1 AND auth_provider_user_id = $2",
    )
    .bind(provider)
    .bind(external_user_id)
    .fetch_optional(pool)
    .await
    .ok()?;

    row.map(|(email,)| (email, Some(true)))
}

/// OIDC userinfo response (subset of fields we need).
#[derive(Debug, Deserialize)]
struct UserinfoResponse {
    email: Option<String>,
    email_verified: Option<bool>,
}

/// Fetch the user's email from the OIDC /userinfo endpoint.
///
/// Auth0 access tokens don't include `email` by default (it requires a custom
/// Action). As a fallback, we call the issuer's `/userinfo` endpoint with the
/// access token to retrieve the email claim.
async fn fetch_email_from_userinfo(
    issuer: &str,
    access_token: &str,
) -> Result<(String, Option<bool>), String> {
    let url = format!("{}userinfo", issuer.trim_end_matches('/').to_owned() + "/");
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
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
