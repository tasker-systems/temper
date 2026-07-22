use crate::{
    act::{Act, ActorAuthority, Provisioner},
    refusal::Refusal,
    standing::Standing,
};

/// Validate one transition against spec §6's table.
///
/// **§6's table is authoritative over §5's diagram.** The earlier diagram showed an edge from
/// `Denied` into `Revoked` that no act produces; in this repo a disagreement between prose and
/// sketch resolves in the sketch's favour, so the table is stated as code here and the diagram is
/// an aid.
///
/// Every cell not listed is illegal and refused **with a reason** — there is no catchall over
/// `Standing`, so adding a state is a compile error here (spec §7 obligation 3).
///
/// # Why every arm enumerates all six cases of `Option<Standing>`
///
/// A `_ =>` arm here would compile fine when a sixth `Standing` variant is added, silently
/// refusing every act from the new state instead of forcing the author to decide each cell. That
/// is precisely the obligation-3 failure this crate boundary exists to prevent, and it would be
/// invisible: fail-closed looks like working software. The verbosity below is the proof.
pub fn transition(
    current: Option<Standing>,
    act: &Act,
    authority: ActorAuthority,
) -> Result<Standing, Refusal> {
    // Authority first. Auth before writes (and before deciding a target state).
    require_authority(act, authority)?;

    match act {
        // Absence only. A returning principal's standing is LOADED, never SET (F4).
        Act::Provision { path } => match current {
            None => Ok(match path {
                Provisioner::BootSeed => Standing::Approved,
                Provisioner::Saml
                | Provisioner::OauthFirstLogin
                | Provisioner::MachineRegistration => Standing::Denied,
            }),
            Some(Standing::Denied)
            | Some(Standing::Requested)
            | Some(Standing::Approved)
            | Some(Standing::Revoked)
            | Some(Standing::Deactivated) => Err(illegal(current, "provision")),
        },

        Act::Request => match current {
            Some(Standing::Denied) => Ok(Standing::Requested),
            None
            | Some(Standing::Requested)
            | Some(Standing::Approved)
            | Some(Standing::Revoked)
            | Some(Standing::Deactivated) => Err(illegal(current, "request")),
        },

        Act::Withdraw => match current {
            Some(Standing::Requested) => Ok(Standing::Denied),
            None
            | Some(Standing::Denied)
            | Some(Standing::Approved)
            | Some(Standing::Revoked)
            | Some(Standing::Deactivated) => Err(illegal(current, "withdraw")),
        },

        // D14 — legal from Denied too: machines have no self and can never Request, so without
        // this the entire machine surface is a dead end.
        // D16 — legal from Revoked: `Reinstate` was identical to this, and the log's prior_state
        // already makes a reinstatement legible.
        Act::Approve => match current {
            Some(Standing::Requested) | Some(Standing::Denied) | Some(Standing::Revoked) => {
                Ok(Standing::Approved)
            }
            None | Some(Standing::Approved) | Some(Standing::Deactivated) => {
                Err(illegal(current, "approve"))
            }
        },

        Act::Reject => match current {
            Some(Standing::Requested) => Ok(Standing::Denied),
            None
            | Some(Standing::Denied)
            | Some(Standing::Approved)
            | Some(Standing::Revoked)
            | Some(Standing::Deactivated) => Err(illegal(current, "reject")),
        },

        // You cannot revoke what was never granted.
        Act::Revoke { .. } => match current {
            Some(Standing::Approved) => Ok(Standing::Revoked),
            None
            | Some(Standing::Denied)
            | Some(Standing::Requested)
            | Some(Standing::Revoked)
            | Some(Standing::Deactivated) => Err(illegal(current, "revoke")),
        },

        // Any LIVE state. Not from absence, and not from Deactivated (already there).
        Act::Deactivate => match current {
            Some(Standing::Denied)
            | Some(Standing::Requested)
            | Some(Standing::Approved)
            | Some(Standing::Revoked) => Ok(Standing::Deactivated),
            None | Some(Standing::Deactivated) => Err(illegal(current, "deactivate")),
        },

        // The only data-dependent target in the machine (spec §6). Refuses rather than guesses.
        Act::Reactivate { prior } => match current {
            Some(Standing::Deactivated) => match prior {
                Some(p) => Ok(*p),
                None => Err(Refusal::NoPriorStanding),
            },
            None
            | Some(Standing::Denied)
            | Some(Standing::Requested)
            | Some(Standing::Approved)
            | Some(Standing::Revoked) => Err(illegal(current, "reactivate")),
        },

        // D15 — sets a marker and moves nothing, so a revocation cannot be laundered back to
        // Denied. The no-laundering property is structural rather than bookkept.
        Act::RequestReview => match current {
            Some(Standing::Revoked) => Ok(Standing::Revoked),
            None
            | Some(Standing::Denied)
            | Some(Standing::Requested)
            | Some(Standing::Approved)
            | Some(Standing::Deactivated) => Err(illegal(current, "request_review")),
        },
    }
}

fn illegal(from: Option<Standing>, act: &'static str) -> Refusal {
    Refusal::IllegalTransition {
        from,
        act: act.to_string(),
    }
}

/// Spec §6's actor column, enforced.
fn require_authority(act: &Act, actual: ActorAuthority) -> Result<(), Refusal> {
    let required = match act {
        // Provision's actor is NOT "none" universally: `temper admin machine provision` is
        // admin-run. Under D11 this grants nothing either way, but the table must not imply an
        // unauthenticated mint path exists — so both Credential and Admin are accepted here.
        Act::Provision { .. } => {
            return match actual {
                ActorAuthority::Credential | ActorAuthority::Admin => Ok(()),
                ActorAuthority::SelfPrincipal => Err(Refusal::InsufficientAuthority {
                    required: ActorAuthority::Credential,
                    actual,
                }),
            }
        }
        Act::Request | Act::Withdraw | Act::RequestReview => ActorAuthority::SelfPrincipal,
        Act::Approve
        | Act::Reject
        | Act::Revoke { .. }
        | Act::Deactivate
        | Act::Reactivate { .. } => ActorAuthority::Admin,
    };

    if actual == required {
        Ok(())
    } else {
        Err(Refusal::InsufficientAuthority { required, actual })
    }
}
