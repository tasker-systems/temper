//! Wire types for the Slack account-link surface.

use serde::{Deserialize, Serialize};

/// The result of a disconnect, as returned to CLI callers.
///
/// Every field is an observation of what actually happened, so the CLI can tell
/// the user the truth rather than echoing a canned success message. In
/// particular `idp_revoked = false` is a normal, non-error outcome: the local
/// unbind is complete either way.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackDisconnectResponse {
    /// An identity row existed and was removed.
    pub was_linked: bool,
    /// A stored grant existed and was destroyed.
    pub grant_deleted: bool,
    /// How many pending link intents were swept.
    pub intents_deleted: i64,
    /// The IdP acknowledged the revocation. `false` means the grant may remain
    /// live at the IdP until it expires; the local copy is destroyed regardless.
    pub idp_revoked: bool,
}

/// Request body for the admin disconnect endpoint.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackDisconnectRequest {
    /// The whole opaque Slack principal (`slack:<team>:<user>`, 2–4 segments).
    /// Never split this value.
    pub slack_principal_id: String,
}
