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
use temper_services::services::standing_service;
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
    let authed = request
        .extensions()
        .get::<AuthenticatedProfile>()
        .ok_or_else(|| {
            ApiError::Internal("AuthenticatedProfile not found in request extensions".to_string())
        })?;

    match temper_services::auth::require_system_access(&state.pool, authed).await {
        Ok(_authorized) => {}
        Err(temper_services::auth::AuthzError::SystemAccessDenied { .. }) => {
            // Surface-side presentation: build the CLI-facing details payload.
            let profile_id = ProfileId::from(authed.profile.id);
            // The typed reason comes straight from the standing machine — no `access_mode` read.
            // `admit` returns `Err(Refusal)` for exactly the state that just failed the gate; the
            // `Ok` arm is only reachable on a race (approved between the gate check and here), in
            // which case a generic denial is the safe fallback.
            let refusal = standing_service::admit(&state.pool, profile_id)
                .await
                .err()
                .unwrap_or(temper_principal::Refusal::NoStanding);
            // SECURITY NOTE: email and display_name are safe to return here because
            // the caller already proved ownership of this identity through OAuth.
            // We are reflecting their own profile data back to them.
            let details = temper_core::types::access_gate::SystemAccessDetails {
                email: authed.profile.email.clone(),
                display_name: Some(authed.profile.display_name.clone()),
                refusal,
                request_url: Some("https://temperkb.io/request-access".to_string()),
                cli_command: Some(
                    temper_core::types::access_gate::REQUEST_ACCESS_COMMAND.to_string(),
                ),
            };
            return Err(ApiError::SystemAccessRequired {
                details: Box::new(details),
            });
        }
        Err(
            temper_services::auth::AuthzError::ProfileResolution(err)
            | temper_services::auth::AuthzError::AccessCheck(err),
        ) => return Err(err),
        Err(temper_services::auth::AuthzError::Deactivated { .. }) => {
            // require_auth already gated deactivation before this layer runs.
            return Err(ApiError::Unauthorized("account is deactivated".to_string()));
        }
        // Level 2 neither classifies a token nor resolves an email — `require_auth`
        // did both before this layer runs. Unreachable from `require_system_access`.
        Err(
            temper_services::auth::AuthzError::Refused(_)
            | temper_services::auth::AuthzError::EmailResolution(_),
        ) => {
            return Err(ApiError::Internal(
                "unexpected authentication error from require_system_access".to_string(),
            ));
        }
    }

    Ok(next.run(request).await)
}
