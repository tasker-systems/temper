//! What an unmatched path answers. Declared beside the route table (`FALLBACK`,
//! `INTERNAL_FALLBACK`, `REFUSED_PREFIXES` in `routes/mod.rs`), never inferred.
//!
//! An unmatched path told an unauthenticated caller which paths exist: a gated route answered 401
//! and an unmatched one 404. Now the unmatched path asks the same question a gated route asks
//! first, so an unauthenticated caller gets the same uniform refusal on both, and only a validly
//! signed caller learns that no route matched.
//!
//! **Lookup-only, never provisioning.** The bearer is verified exactly as `require_auth` verifies it
//! (`middleware::auth::verify_bearer`), and the profile is resolved with
//! `authenticate_caller_existing_only`, never `authenticate_caller` or `require_auth`, which
//! auto-provision a profile and confer auto-join team reach. A mistyped URL must not mint an
//! account. The cost per unmatched authenticated request is one JWT verification and one profile
//! lookup, plus the email ladder (and possibly one `/userinfo` call) when the token carries no
//! email claim and no link is cached. No write. Reaching the ladder takes a validly signed token,
//! so it is no lever for an anonymous caller.

use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};

use temper_services::auth::{AuthzError, NO_EXISTING_PROFILE};
use temper_services::error::{ApiError, UnauthCause};
use temper_services::state::AppState;
use temper_services::transport::fallback_handler;

use crate::middleware::auth::verify_bearer;

use super::REFUSED_PREFIXES;

/// Whether `path` is under a prefix every unmatched request is refused on, whoever sends it.
pub(super) fn under_refused_prefix(path: &str) -> bool {
    REFUSED_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// `create_app`'s fallback: the uniform refusal unless the caller holds a validly signed bearer,
/// then the 404.
pub(super) async fn lookup_then_not_found(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    if under_refused_prefix(request.uri().path()) {
        return refused_prefix().await;
    }
    let (token, raw) = match verify_bearer(&state, request.headers()).await {
        Ok(verified) => verified,
        // Every way a bearer can fail, its absence included, is the one refusal. Its cause is
        // logged on the outer service's `auth error` line; the key store's outage is answered
        // there too, for every refusal at once.
        Err(_) => return ApiError::Unauthenticated(UnauthCause::UnmatchedPath).into_response(),
    };
    // Every result below is a validly signed caller, so none is anonymous: each keeps the answer
    // the same caller gets on a bearer route, or the 404.
    match temper_services::auth::authenticate_caller_existing_only(&state, &raw, &token).await {
        Ok(_) => fallback_handler(request).await,
        Err(AuthzError::Deactivated { profile_id }) => {
            tracing::warn!(%profile_id, "rejected: profile is deactivated");
            ApiError::Unauthorized("account is deactivated".to_string()).into_response()
        }
        Err(AuthzError::Refused(why)) if why == NO_EXISTING_PROFILE => {
            fallback_handler(request).await
        }
        Err(AuthzError::Refused(why)) => {
            ApiError::Unauthorized(format!("machine credential refused: {why}")).into_response()
        }
        Err(AuthzError::EmailResolution(err)) => err.into_response(),
        Err(_) => fallback_handler(request).await,
    }
}

/// `create_internal_app`'s fallback, and `create_app`'s under a refused prefix: the uniform refusal,
/// unconditionally. No bearer route lives there, so there is nothing a bearer could reach.
pub(super) async fn refused_prefix() -> Response {
    ApiError::Unauthenticated(UnauthCause::RefusedPrefix).into_response()
}
