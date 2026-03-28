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

/// Newtype carrying the value of the `X-Temper-Client-Id` request header.
#[derive(Debug, Clone)]
pub struct ClientId(pub String);

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

    // 4. Build AuthClaims.
    let email = token_data
        .claims
        .email
        .ok_or_else(|| ApiError::Unauthorized("Token missing required email claim".to_string()))?;

    let claims = AuthClaims {
        provider: "neon_auth".to_string(),
        external_user_id: token_data.claims.sub,
        email,
        email_verified: token_data.claims.email_verified,
        exp: token_data.claims.exp,
        iat: token_data.claims.iat,
    };

    // 5. Resolve (or auto-provision) the profile.
    let profile = profile_service::resolve_from_claims(&state.pool, &claims).await?;

    // 6. Optionally capture X-Temper-Client-Id.
    let client_id = request
        .headers()
        .get("X-Temper-Client-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| ClientId(s.to_string()));

    if let Some(id) = client_id {
        request.extensions_mut().insert(id);
    }

    // 7. Inject AuthenticatedProfile into extensions.
    request
        .extensions_mut()
        .insert(AuthenticatedProfile { profile, claims });

    // 8. Continue.
    Ok(next.run(request).await)
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
