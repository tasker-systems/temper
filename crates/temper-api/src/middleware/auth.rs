use axum::body::Body;
use axum::extract::{FromRequestParts, State};
use axum::http::{request::Parts, Request};
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{decode, TokenData};
use std::future::Future;

use temper_core::types::AuthenticatedProfile;

use temper_services::error::ApiError;
use temper_services::state::AppState;

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

    // 2. Get the current decoding key (and its algorithm family) from the JWKS store (cached).
    let vk = state.jwks_store.get_decoding_key().await.map_err(|e| {
        tracing::error!("JWKS key retrieval failed: {e}");
        ApiError::Unauthorized("Authentication service unavailable".to_string())
    })?;

    // 3. Decode and verify the JWT. The allow-list is scoped to exactly the
    //    loaded key's algorithm (see `JwksKeyStore::validation`).
    let issuer = &state.config.auth_issuer;
    let audience = state.config.auth_audience.as_deref();
    let validation = state.jwks_store.validation(issuer, audience, vk.algorithm);

    let token_data: TokenData<temper_services::auth::RawJwtClaims> =
        decode(&token, &vk.key, &validation).map_err(|e| {
            tracing::debug!("JWT verification failed: {e}");
            ApiError::Unauthorized("Invalid or expired token".to_string())
        })?;
    let raw = token_data.claims;

    // 4. Authenticate through the shared seam: classification, the human email
    //    ladder (token claim → cached link → OIDC /userinfo), claim construction and
    //    the deactivation gate all live in temper-services::auth. This surface no
    //    longer builds an `AuthClaims` — it hands over a verified token and maps the
    //    refusal vocabulary to HTTP. That is what keeps the two surfaces from
    //    answering "who is this human" differently.
    let authed = temper_services::auth::authenticate_token(&state, &raw, &token)
        .await
        .map_err(|e| match e {
            // The seam has already logged the reason with the `sub`; on the wire this
            // is indistinguishable from any other bad token, as it was before.
            temper_services::auth::AuthzError::Refused(_) => {
                ApiError::Unauthorized("Invalid or expired token".to_string())
            }
            temper_services::auth::AuthzError::Deactivated { profile_id } => {
                tracing::warn!(%profile_id, "rejected: profile is deactivated");
                ApiError::Unauthorized("account is deactivated".to_string())
            }
            temper_services::auth::AuthzError::EmailResolution(err)
            | temper_services::auth::AuthzError::ProfileResolution(err) => err,
            // Level 1 never runs the system-access gate; these are defensively
            // unreachable from `authenticate_token`.
            temper_services::auth::AuthzError::AccessCheck(_)
            | temper_services::auth::AuthzError::SystemAccessDenied { .. } => {
                ApiError::Internal("unexpected system-access error from authenticate".to_string())
            }
        })?;
    let profile = authed.profile.clone();

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
    request.extensions_mut().insert(authed);

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
