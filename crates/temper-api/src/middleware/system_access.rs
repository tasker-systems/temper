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

use temper_core::types::ids::ProfileId;
use temper_core::types::AuthenticatedProfile;

use temper_services::error::ApiError;
use temper_services::services::access_service;
use temper_services::state::AppState;

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

    let has_access =
        access_service::has_system_access(&state.pool, ProfileId::from(profile.profile.id)).await?;

    if !has_access {
        let settings = access_service::get_public_settings(&state.pool).await?;
        let own_request =
            access_service::get_own_request(&state.pool, ProfileId::from(profile.profile.id))
                .await?;

        // SECURITY NOTE: email and display_name are safe to return here because
        // the caller already proved ownership of this identity through OAuth.
        // We are reflecting their own profile data back to them.
        let details = temper_core::types::access_gate::SystemAccessDetails {
            email: profile.profile.email.clone(),
            display_name: Some(profile.profile.display_name.clone()),
            access_mode: settings.access_mode,
            join_request_status: own_request.map(|r| r.status),
            request_url: Some("https://temperkb.io/request-access".to_string()),
            cli_command: Some("temper team join --message \"...\"".to_string()),
        };
        return Err(ApiError::SystemAccessRequired {
            details: Box::new(details),
        });
    }

    Ok(next.run(request).await)
}
