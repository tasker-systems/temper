//! Middleware that enforces system-level access.
//!
//! Applied to the gated router — all routes that require the caller to be
//! an approved member of the gating team. Routes in the auth-only router
//! (profile, access endpoints) bypass this middleware entirely via the
//! router split in routes.rs.

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use temper_core::types::AuthenticatedProfile;

use crate::error::ApiError;
use crate::services::access_service;
use crate::state::AppState;

/// Axum middleware that checks system-level access after authentication.
///
/// Reads `AuthenticatedProfile` from request extensions (set by `require_auth`)
/// and calls `has_system_access`. Returns `SystemAccessRequired` if the profile
/// is not an approved member of the gating team.
pub async fn require_system_access(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let profile = request
        .extensions()
        .get::<AuthenticatedProfile>()
        .ok_or_else(|| {
            ApiError::Internal("AuthenticatedProfile not found in request extensions".to_string())
        })?;

    let has_access = access_service::has_system_access(&state.pool, profile.profile.id).await?;

    if !has_access {
        return Err(ApiError::SystemAccessRequired);
    }

    Ok(next.run(request).await)
}
