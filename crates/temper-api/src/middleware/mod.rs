pub mod auth;
pub mod internal_auth;
pub mod relay_trust;
pub mod surface;
pub mod system_access;

use temper_services::auth::Caller;

/// The request extension `auth::require_auth` plants: the classified caller, and nothing else.
///
/// **Private to this module**, so only the middleware and its extractors can name it. A handler
/// cannot read the caller's identity except through `auth::AuthUser` (a person) or
/// `auth::AnyPrincipal` (an explicit opt-in that admits a machine) — there is no raw
/// `AuthenticatedProfile` extension left to reach past them.
#[derive(Clone)]
struct Planted(Caller);
