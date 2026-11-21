//! The per-principal rate-limit layer: the seam's budget for a door keyed on the verified
//! caller. The budget, its counter and its refusal live in `temper_services::rate_limit`
//! ([`check_principal`]); this layer only reads the caller `require_auth` planted and asks.
//!
//! It must sit inner to `require_auth`, and on the gated tiers inner to
//! `require_system_access` too, so an unauthenticated or unapproved caller is refused by
//! its gate and never spends a budget. `routes::mount_group` adds it before the tier stack,
//! which places it there.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use temper_core::types::ids::ProfileId;
use temper_services::error::ApiError;
use temper_services::rate_limit::{check_principal, PrincipalDoor};
use temper_services::state::AppState;

/// The layer's state: the app state and the door whose budget the routes spend.
#[derive(Clone)]
pub struct PrincipalLimit {
    pub state: AppState,
    pub door: PrincipalDoor,
}

/// Count the call against the caller's budget on `door`, then run the route, or refuse it
/// with the seam's `429` past the operator's limit.
///
/// No limit configured ⇒ pass-through before anything else is read, so default off is the
/// route exactly as it is without the layer. With a limit and no planted caller the layer
/// was mounted outside `require_auth`: a wiring error, answered `500` rather than passed
/// unlimited.
pub async fn require_principal_rate_limit(
    State(limit): State<PrincipalLimit>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let config = limit.state.config.rate_limit.as_ref();
    if limit.door.limit(config).is_none() {
        return Ok(next.run(request).await);
    }

    let profile_id = super::auth::planted_caller(&request)
        .map(|caller| ProfileId::from(caller.profile().id))
        .ok_or_else(|| ApiError::Internal("caller not found in request extensions".to_string()))?;

    check_principal(&limit.state.pool, config, limit.door, profile_id).await?;
    Ok(next.run(request).await)
}
