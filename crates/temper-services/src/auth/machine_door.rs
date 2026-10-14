//! Which machine-credential acts an instance permits, by its [`AuthMode`].
//!
//! Each deployment trusts exactly one token issuer (`state.rs` validation sets one `iss`), and the
//! mode names it: an external IdP (Auth0) in [`AuthMode::ExternalIdp`], temper's own authorization
//! server in [`AuthMode::TemperAs`]. A `kb_machine_clients` row records which of the two minted its
//! credential (`issuer`). So the mode decides two things, and this module is the one place that
//! says both:
//!
//! - **The gate.** A machine token can legitimately reach only rows its issuer mints
//!   ([`reachable_issuer`]). An `auth0-m2m` row on an AS-mode instance can be matched by no token
//!   the AS issues, so a token that does match one is a capture. The same holds the other way
//!   round.
//! - **The doors.** A door that writes a credential the instance can never authenticate breaks
//!   the machine instead of registering it ([`machine_door_policy`]).
//!
//! | act | `ExternalIdp` | `TemperAs` |
//! |---|---|---|
//! | gate resolves | `auth0-m2m` rows | `temper` rows |
//! | `provision` | system admin only | refused |
//! | `issue` | allowed, dormant until AS mode | allowed |
//! | `rebind` | allowed (`auth0-m2m` source rows) | refused |
//!
//! Ruled 2026-10-13 (task `01a13297`), overturning the issuer-agnostic lookup recorded as D4 in
//! `internal/auth/machine-token-contract.md`. The table seeds a fuller boot-resolved deployment
//! mode (task `01a13664`), which adds the upstream identity provider as a second axis.

use crate::auth_config::AuthMode;

/// `kb_machine_clients.issuer` for a credential an external IdP minted.
pub const EXTERNAL_IDP_ISSUER: &str = "auth0-m2m";

/// `kb_machine_clients.issuer` for a credential temper's own authorization server minted.
pub const TEMPER_ISSUER: &str = "temper";

/// The one `kb_machine_clients.issuer` whose rows a machine token can reach on an instance in
/// `mode`.
pub fn reachable_issuer(mode: AuthMode) -> &'static str {
    match mode {
        AuthMode::ExternalIdp => EXTERNAL_IDP_ISSUER,
        AuthMode::TemperAs => TEMPER_ISSUER,
    }
}

/// A door that writes a machine credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineDoor {
    /// Register an externally issued client id (`issuer = auth0-m2m`).
    Provision,
    /// Mint a temper client id and secret (`issuer = temper`).
    Issue,
    /// Point a fresh externally issued client id at an existing machine's profile.
    Rebind,
}

/// What an instance's mode permits at a door.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorPolicy {
    /// Open to every caller the door's own authorization admits.
    Allowed,
    /// Open to a system admin only.
    AdminOnly,
    /// Allowed, but the credential will not authenticate on this instance until it runs temper's
    /// own authorization server. The response says so.
    Dormant,
    /// Refused: the credential could never authenticate here. The reason names the door to use
    /// instead.
    Refused(&'static str),
}

/// The whole door table: what an instance in `mode` permits at `door`.
///
/// `provision` is admin-only under an external IdP because only the IdP's administrator can
/// create a client worth registering, and a team owner registering one they found would capture
/// its tokens (ruling 3). `issue` stays open there, dormant, so credentials can be staged before
/// an instance moves to its own authorization server.
pub fn machine_door_policy(mode: AuthMode, door: MachineDoor) -> DoorPolicy {
    match (mode, door) {
        (AuthMode::ExternalIdp, MachineDoor::Provision) => DoorPolicy::AdminOnly,
        (AuthMode::ExternalIdp, MachineDoor::Issue) => DoorPolicy::Dormant,
        (AuthMode::ExternalIdp, MachineDoor::Rebind) => DoorPolicy::Allowed,
        (AuthMode::TemperAs, MachineDoor::Provision) => DoorPolicy::Refused(
            "this instance mints its own machine credentials, so an externally issued client id \
             could never authenticate here; issue one with `temper admin machine issue`",
        ),
        (AuthMode::TemperAs, MachineDoor::Issue) => DoorPolicy::Allowed,
        (AuthMode::TemperAs, MachineDoor::Rebind) => DoorPolicy::Refused(
            "this instance mints its own machine credentials, so an externally issued client id \
             could never authenticate here; rotate a temper-issued secret with \
             `temper admin machine rotate-secret`, or issue a new credential with \
             `temper admin machine issue`",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_reaches_only_the_issuer_the_mode_trusts() {
        assert_eq!(reachable_issuer(AuthMode::ExternalIdp), "auth0-m2m");
        assert_eq!(reachable_issuer(AuthMode::TemperAs), "temper");
        // The external issuer tag is the one the machine seam stamps on every machine token.
        assert_eq!(EXTERNAL_IDP_ISSUER, crate::auth::MACHINE_PROVIDER_TAG);
    }

    #[test]
    fn the_door_table_is_the_ruled_one() {
        use DoorPolicy::*;
        use MachineDoor::*;
        let external = |door| machine_door_policy(AuthMode::ExternalIdp, door);
        let temper_as = |door| machine_door_policy(AuthMode::TemperAs, door);

        assert_eq!(external(Provision), AdminOnly);
        assert_eq!(external(Issue), Dormant);
        assert_eq!(external(Rebind), Allowed);

        assert!(matches!(temper_as(Provision), Refused(why) if why.contains("machine issue")));
        assert_eq!(temper_as(Issue), Allowed);
        assert!(matches!(temper_as(Rebind), Refused(why) if why.contains("rotate-secret")));
    }
}
