//! Wire types for the Slack account-link surface.

use serde::{Deserialize, Serialize};

/// What happened to the stored grant at the identity provider.
///
/// A three-state enum rather than a `bool`, because `false` used to collapse
/// three genuinely different facts — "there was no grant, so nothing was
/// attempted", "a revoke was attempted and failed", and (in AS mode) "the
/// UPDATE matched zero rows" — and consumers could not tell them apart. The CLI
/// consequently warned "the identity provider did not confirm revocation" at a
/// user who had no grant at all.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdpRevocation {
    /// No stored grant, so no revocation was attempted.
    NotAttempted,
    /// The IdP (or, in AS mode, the local token store) confirmed revocation.
    Revoked,
    /// A revocation was attempted and did not succeed. The local grant was
    /// destroyed regardless; the grant may remain live at the IdP.
    Failed,
}

/// One principal that a disconnect actually unbound.
///
/// Every field is an observation of what happened to THAT principal, so the CLI
/// can tell the user the truth rather than echoing a canned success message.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackDisconnectedPrincipal {
    /// The WHOLE opaque Slack principal that was unbound. Never split.
    pub slack_principal_id: String,
    /// A stored grant existed and was destroyed.
    pub grant_deleted: bool,
    /// How many pending link intents were swept for this principal.
    pub intents_deleted: i64,
    /// What happened to the grant at the identity provider.
    pub idp_revocation: IdpRevocation,
}

/// The result of a disconnect, as returned to CLI callers.
///
/// Both surfaces return this same shape: the admin arm carries 0 or 1 entries,
/// the self-serve arm 0..n (a human legitimately holds one Slack principal per
/// workspace, and `kb_profile_auth_links` carries no `UNIQUE(profile_id,
/// auth_provider)` that would stop them). Uniform, so an SDK consumer writes one
/// code path for both.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackDisconnectResponse {
    /// One entry per principal actually unbound. Empty when nothing was linked —
    /// which is a success, not an error: disconnect is idempotent.
    pub disconnected: Vec<SlackDisconnectedPrincipal>,
}

/// Request body for the admin disconnect endpoint.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackDisconnectRequest {
    /// The whole opaque Slack principal (`slack:<team>:<user>`, 2–4 segments).
    /// Never split this value.
    pub slack_principal_id: String,
}
