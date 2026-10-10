//! Authenticated but NOT system-access-gated — profile and self-service access
//! endpoints. Documented (a caller managing their own instance is a library
//! caller, not an operator).
//!
//! Two groups, two tiers. `auth_only_routes` is a person's self-service and refuses a machine at
//! the tier; `auth_only_status_routes` is the profile read a machine keeps.

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::handlers;
use temper_services::state::AppState;

pub(super) fn auth_only_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::profiles::update))
        .routes(routes!(handlers::profiles::list_auth_links))
        .routes(routes!(handlers::access::create_request))
        .routes(routes!(
            handlers::access::get_own_request,
            handlers::access::withdraw_request
        ))
        .routes(routes!(handlers::access::create_review_request))
        .routes(routes!(handlers::access::get_settings))
        .routes(routes!(handlers::invitations::list_mine))
        .routes(routes!(handlers::invitations::count_mine))
        .routes(routes!(handlers::invitations::accept))
        .routes(routes!(handlers::invitations::decline))
        .routes(routes!(handlers::slack_disconnect::disconnect_me))
}

/// The caller's own profile and entitlements — the one self-service read open to a machine.
pub(super) fn auth_only_status_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::profiles::get))
}
