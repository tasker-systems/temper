//! `MachinePrincipal` is sealed exactly as `HumanPrincipal` is: wrapping an `AuthenticatedProfile`
//! in one outside `temper_services::auth` must not compile.
use temper_services::auth::{AuthenticatedProfile, MachinePrincipal};

fn forge(authed: AuthenticatedProfile) -> MachinePrincipal {
    // E0423: cannot initialize a tuple struct which contains private fields.
    MachinePrincipal(authed)
}

fn main() {}
