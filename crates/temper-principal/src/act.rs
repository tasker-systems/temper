use crate::standing::Standing;
use serde::{Deserialize, Serialize};

/// Which door minted the principal (spec §6's provision table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provisioner {
    Saml,
    OauthFirstLogin,
    MachineRegistration,
    /// Genesis. The one deliberate exception that births `Approved` (D11, F6): on a fresh
    /// instance no admin exists, so nobody could ever be approved. Bootstrapping temper already
    /// requires database write access, and the bootstrap SoP foregrounds that.
    BootSeed,
}

/// Who is acting. Three authorities (spec §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorAuthority {
    /// The credential itself is the authority — provision paths only.
    Credential,
    /// The principal acting on its own standing.
    SelfPrincipal,
    /// An actor for whom `is_system_admin` holds.
    Admin,
}

/// The eight acts (D16 dropped `Reinstate`), plus `RequestReview` which moves nothing (D15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "act")]
pub enum Act {
    /// Fires **only on profile mint**, never on a returning principal. An existing auth link
    /// returns at step 1 of `resolve_human_from_claims` and never reaches the mint; a returning
    /// principal's standing is *loaded, not set*. This is what closes F4 structurally.
    Provision {
        path: Provisioner,
    },
    /// The consent-capturing act (D12) — human-only. Machines have no self and can never Request.
    Request,
    Withdraw,
    Approve,
    Reject,
    Revoke {
        reason: String,
    },
    Deactivate,
    /// The only data-dependent target in the machine (spec §6). `prior` is read from the standing
    /// log by the caller; `None` refuses rather than guesses.
    Reactivate {
        prior: Option<Standing>,
    },
    /// Sets a marker and moves nothing (D15). **Never an admission input** — see `admission.rs`.
    RequestReview,
}
