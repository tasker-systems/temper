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
// NOT a doc comment, deliberately: `ToSchema` publishes the doc comment above as this type's
// `description` in openapi.json, and from there into the generated Ruby gem and temper-ts's
// schema.ts. Build-system rationale is not part of the API contract an SDK consumer reads, and
// writing it there restales three committed artifacts for no reader's benefit.
//
// The `scenario-schema` derive lets this type appear in a ledger payload
// (`temper_substrate::payloads::SlackPrincipalDisconnected::idp_revocation`) instead of the payload
// mirroring a copy of these three variants. It is the same type the HTTP surface returns, so the
// ledger and the API cannot disagree about what "revoked" means — the repo's shared-types-at-
// boundaries rule, applied to an event payload rather than a wire DTO.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
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

impl IdpRevocation {
    /// The wire spelling, exactly as the `#[serde(rename_all = "snake_case")]` above produces it.
    ///
    /// Exists because the ledger writer hands this to a plpgsql function as a bare `text` bind
    /// (`_admin_slack_disconnected`), and the registry's `payload_schema` validates the result
    /// against these three literals. Deliberately NOT `serde_json::to_string` — that yields a
    /// *quoted* `"revoked"`, which matches no enum variant on the way back in, and it launders a
    /// typed enum through a string round-trip to reach a value the type already knows. Same
    /// reasoning as `temper_substrate::payloads::AnchorTable::as_str`.
    ///
    /// No `_ =>` arm: a new variant must be a compile error here, not a silent wrong spelling in an
    /// append-only audit record.
    pub fn as_str(self) -> &'static str {
        match self {
            IdpRevocation::NotAttempted => "not_attempted",
            IdpRevocation::Revoked => "revoked",
            IdpRevocation::Failed => "failed",
        }
    }
}

#[cfg(test)]
mod idp_revocation_tests {
    use super::IdpRevocation;

    /// `as_str` and serde must not drift: the ledger writes via `as_str` and the payload is read
    /// back through `Deserialize`, so a mismatch would produce audit rows that cannot deserialize
    /// into the very struct that documents them.
    #[test]
    fn as_str_matches_the_serde_rename() {
        for v in [
            IdpRevocation::NotAttempted,
            IdpRevocation::Revoked,
            IdpRevocation::Failed,
        ] {
            let via_serde = serde_json::to_string(&v).expect("serialize");
            assert_eq!(
                via_serde.trim_matches('"'),
                v.as_str(),
                "as_str disagrees with the serde rename for {v:?}"
            );
        }
    }
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
