//! `AuthenticatedProfile` has private fields — constructing one by struct literal outside
//! `temper_services::auth` must not compile. This is the seal at Level 1, the rung the other two
//! fixtures stand on: `SystemAuthorized` and `SystemAdmin` are both minted from an
//! `AuthenticatedProfile`, so a forgeable Level 1 would have made sealing them decorative.
//!
//! It lived in temper-core with `pub` fields until 2026-07-22, which is exactly what this forgery
//! used to be: legal, silent, and available to every crate in the workspace.
use temper_core::types::{AuthClaims, Profile};
use temper_services::auth::AuthenticatedProfile;

// Takes the parts by argument rather than building them, so the only diagnostic is the seal itself
// — a fixture that also failed to construct a `Profile` would prove nothing about privacy.
fn forge(profile: Profile, claims: AuthClaims) -> AuthenticatedProfile {
    // E0451: fields `profile` and `claims` of `AuthenticatedProfile` are private.
    AuthenticatedProfile { profile, claims }
}

fn main() {}
