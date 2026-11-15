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
//! email claim and no link is cached. It creates no profile; like sign-in, it may refresh an auth
//! link's verification, attach a verified email's link to the profile that already holds it, or
//! touch a registered machine's `last_seen_at` (at most once per five minutes).
//! Reaching any of it takes a validly signed token, so it is no lever for an anonymous caller.
//!
//! A signed caller gets the answer a gated route gives the same token when that answer is a refusal
//! about the credential (deactivated, an unregistered or revoked machine, an email that cannot be
//! resolved) or a server fault, and otherwise the 404.

use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};

use temper_services::auth::{AuthzError, NO_EXISTING_PROFILE};
use temper_services::error::{ApiError, UnauthCause};
use temper_services::state::AppState;
use temper_services::transport::fallback_handler;

use crate::middleware::auth::verify_bearer;

use super::REFUSED_PREFIXES;

/// Whether `path` is under a prefix every unmatched request is refused on, whoever sends it.
///
/// Leading slashes are collapsed first, and the prefix's own bare form (`/internal`) counts, so
/// `//internal/x` and `/internal` are refused like `/internal/x`. None of them routes to a door, but
/// the rule is "every unmatched path under the prefix", and those are spellings of it. Matching is
/// on bytes, as routing is: `/INTERNAL/x` is a different path, unmatched like any other.
pub(super) fn under_refused_prefix(path: &str) -> bool {
    let collapsed = format!("/{}", path.trim_start_matches('/'));
    REFUSED_PREFIXES
        .iter()
        .any(|prefix| collapsed.starts_with(prefix) || collapsed == prefix.trim_end_matches('/'))
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
        // Every way a bearer can fail, its absence included, is the one refusal, logged on the
        // outer service's `auth error` line as `unmatched_path`. The bearer's own cause is kept
        // here, at debug, for an operator who needs it; the key store's outage is answered by the
        // outer service for every refusal at once.
        Err(err) => {
            if let ApiError::Unauthenticated(cause) = &err {
                tracing::debug!(
                    bearer_cause = cause.as_str(),
                    "unmatched path: bearer refused"
                );
            }
            return ApiError::Unauthenticated(UnauthCause::UnmatchedPath).into_response();
        }
    };
    // The token verified, so this caller is one of ours: join its trace, as `require_auth` does.
    temper_telemetry::link_trusted_caller(&tracing::Span::current(), request.headers());
    // Every result below is a validly signed caller, so none is anonymous: each keeps the answer
    // the same caller gets on a bearer route, or the 404.
    match temper_services::auth::authenticate_caller_existing_only(&state, &raw, &token).await {
        Ok(caller) => {
            tracing::Span::current()
                .record("profile_id", tracing::field::display(caller.profile().id));
            fallback_handler(request).await
        }
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
        // The answers `require_auth` gives the same token: an email that cannot be resolved, an
        // unregistered or revoked machine, or a server fault, never hidden behind a 404.
        Err(AuthzError::EmailResolution(err))
        | Err(AuthzError::ProfileResolution(err))
        | Err(AuthzError::Classification(err)) => err.into_response(),
        Err(AuthzError::AccessCheck(_)) | Err(AuthzError::SystemAccessDenied { .. }) => {
            ApiError::Internal("unexpected system-access error from a lookup".to_string())
                .into_response()
        }
    }
}

/// `create_internal_app`'s fallback, and `create_app`'s under a refused prefix: the uniform refusal,
/// unconditionally. No bearer route lives there, so there is nothing a bearer could reach.
pub(super) async fn refused_prefix() -> Response {
    ApiError::Unauthenticated(UnauthCause::RefusedPrefix).into_response()
}
