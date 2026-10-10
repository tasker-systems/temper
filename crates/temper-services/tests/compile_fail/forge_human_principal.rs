//! `HumanPrincipal` has a private field — wrapping an `AuthenticatedProfile` in one outside
//! `temper_services::auth` must not compile. The classification is its only constructor, so holding
//! one means the caller was classified a person; an unclassified profile cannot be promoted to one.
use temper_services::auth::{AuthenticatedProfile, HumanPrincipal};

fn forge(authed: AuthenticatedProfile) -> HumanPrincipal {
    // E0423: cannot initialize a tuple struct which contains private fields.
    HumanPrincipal(authed)
}

fn main() {}
