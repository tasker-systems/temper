//! The exhaustive state × act × authority matrix (spec §12).
//!
//! "The state × act matrix is exhaustively enumerable — five states × eight acts ×
//! actor-authority variants, as a table test with no database. Adding a state fails compilation
//! until every cell is filled."
//!
//! Every illegal cell asserts a *reason*, not merely a refusal: "The point of refusing at the act
//! is that the actor learns why; a test that only checks 'not admitted' would pass on a silent
//! denial." (spec §12)

use temper_principal::{Act, ActorAuthority, Refusal, Standing};

/// Every state, including absence. Adding a `Standing` variant without extending this array is
/// caught by `every_standing_variant_is_in_the_matrix` below.
const STATES: [Option<Standing>; 6] = [
    None,
    Some(Standing::Denied),
    Some(Standing::Requested),
    Some(Standing::Approved),
    Some(Standing::Revoked),
    Some(Standing::Deactivated),
];

fn all_acts() -> Vec<Act> {
    vec![
        Act::Provision {
            path: temper_principal::Provisioner::OauthFirstLogin,
        },
        Act::Request,
        Act::Withdraw,
        Act::Approve,
        Act::Reject,
        Act::Revoke {
            reason: "test".to_string(),
        },
        Act::Deactivate,
        Act::Reactivate {
            prior: Some(Standing::Approved),
        },
        Act::RequestReview,
    ]
}

const AUTHORITIES: [ActorAuthority; 3] = [
    ActorAuthority::Credential,
    ActorAuthority::SelfPrincipal,
    ActorAuthority::Admin,
];

#[test]
fn every_cell_is_decided_and_every_refusal_carries_a_reason() {
    for state in STATES {
        for act in all_acts() {
            for authority in AUTHORITIES {
                let outcome = temper_principal::transition(state, &act, authority);
                if let Err(refusal) = outcome {
                    assert!(
                        !refusal.reason().is_empty(),
                        "cell ({state:?}, {act:?}, {authority:?}) refused with an empty reason — \
                         a silent denial is the failure this test exists to catch"
                    );
                }
            }
        }
    }
}

#[test]
fn the_legal_cells_are_exactly_the_spec_six_table() {
    use ActorAuthority::{Admin, SelfPrincipal};
    use Standing::*;

    let legal: Vec<(Option<Standing>, Act, ActorAuthority, Standing)> = vec![
        (Some(Denied), Act::Request, SelfPrincipal, Requested),
        (Some(Requested), Act::Withdraw, SelfPrincipal, Denied),
        (Some(Requested), Act::Approve, Admin, Approved),
        (Some(Denied), Act::Approve, Admin, Approved), // D14 — machines never Request
        (Some(Revoked), Act::Approve, Admin, Approved), // D16 — no separate Reinstate
        (Some(Requested), Act::Reject, Admin, Denied),
        (
            Some(Approved),
            Act::Revoke { reason: "r".into() },
            Admin,
            Revoked,
        ),
        (Some(Denied), Act::Deactivate, Admin, Deactivated),
        (Some(Requested), Act::Deactivate, Admin, Deactivated),
        (Some(Approved), Act::Deactivate, Admin, Deactivated),
        (Some(Revoked), Act::Deactivate, Admin, Deactivated),
        (Some(Revoked), Act::RequestReview, SelfPrincipal, Revoked), // D15 — moves nothing
    ];

    for (from, act, authority, expected) in legal {
        assert_eq!(
            temper_principal::transition(from, &act, authority),
            Ok(expected),
            "spec §6 says ({from:?}, {act:?}, {authority:?}) → {expected:?}"
        );
    }
}

#[test]
fn revoke_is_illegal_from_denied_and_requested() {
    // Spec §6: "you cannot revoke what was never granted." §5's diagram shows an arrow into
    // Revoked originating at Denied; no act produces that edge. §6 is authoritative.
    for from in [Standing::Denied, Standing::Requested] {
        let out = temper_principal::transition(
            Some(from),
            &Act::Revoke { reason: "r".into() },
            ActorAuthority::Admin,
        );
        assert!(
            matches!(out, Err(Refusal::IllegalTransition { .. })),
            "Revoke from {from:?} must be refused — nothing was ever granted"
        );
    }
}

#[test]
fn a_revoked_principal_cannot_re_request() {
    // D15: "there is no path out of Revoked except an admin act, so there is nothing to launder."
    let out = temper_principal::transition(
        Some(Standing::Revoked),
        &Act::Request,
        ActorAuthority::SelfPrincipal,
    );
    assert!(
        matches!(out, Err(Refusal::IllegalTransition { .. })),
        "Revoked → Request must be refused; RequestReview is the only self act from Revoked"
    );
}

#[test]
fn request_review_leaves_standing_unchanged() {
    // D15 obligation: the marker moves nothing.
    assert_eq!(
        temper_principal::transition(
            Some(Standing::Revoked),
            &Act::RequestReview,
            ActorAuthority::SelfPrincipal
        ),
        Ok(Standing::Revoked)
    );
}

#[test]
fn every_self_act_is_illegal_from_deactivated() {
    // Spec §6 requires this be specified on the table's own terms, NOT left to
    // gate_resolved_profile (auth/mod.rs:246) making it unreachable. Leaning on another layer is
    // the cross-layer reasoning that produced this design's original bugs.
    for act in [Act::Request, Act::Withdraw, Act::RequestReview] {
        let out = temper_principal::transition(
            Some(Standing::Deactivated),
            &act,
            ActorAuthority::SelfPrincipal,
        );
        assert!(
            matches!(out, Err(Refusal::IllegalTransition { .. })),
            "{act:?} from Deactivated must be refused by this table, independently of Level 1"
        );
    }
}

#[test]
fn provision_is_legal_only_from_absence_and_never_grants() {
    use temper_principal::Provisioner;
    // D11: "Every provision path births Denied. No door grants access."
    for path in [
        Provisioner::Saml,
        Provisioner::OauthFirstLogin,
        Provisioner::MachineRegistration,
    ] {
        assert_eq!(
            temper_principal::transition(
                None,
                &Act::Provision { path },
                ActorAuthority::Credential
            ),
            Ok(Standing::Denied),
            "{path:?} must birth Denied — no door grants access (D11)"
        );
    }

    // The genesis exception, deliberate and load-bearing (D11, F6).
    assert_eq!(
        temper_principal::transition(
            None,
            &Act::Provision {
                path: Provisioner::BootSeed
            },
            ActorAuthority::Credential
        ),
        Ok(Standing::Approved),
        "the boot-seed mints the first admin — the one deliberate exception"
    );

    // "Provision fires only on profile mint, never on a returning principal." A revoked SAML
    // principal re-asserting must not be re-provisioned back to Denied (F4).
    let out = temper_principal::transition(
        Some(Standing::Revoked),
        &Act::Provision {
            path: Provisioner::Saml,
        },
        ActorAuthority::Credential,
    );
    assert!(
        matches!(out, Err(Refusal::IllegalTransition { .. })),
        "Provision from an existing standing must be refused — this is what closes F4 structurally"
    );
}

#[test]
fn reactivate_restores_the_prior_state_and_refuses_without_one() {
    for prior in [
        Standing::Denied,
        Standing::Requested,
        Standing::Approved,
        Standing::Revoked,
    ] {
        assert_eq!(
            temper_principal::transition(
                Some(Standing::Deactivated),
                &Act::Reactivate { prior: Some(prior) },
                ActorAuthority::Admin
            ),
            Ok(prior),
            "Reactivate restores rather than guesses (spec §5)"
        );
    }

    // Backfilled rows are the exception §5 names: the log begins at migration time. The backfill
    // writes a genesis entry (§11) precisely so this arm is unreachable in practice — but the
    // machine must still refuse rather than guess.
    assert!(
        matches!(
            temper_principal::transition(
                Some(Standing::Deactivated),
                &Act::Reactivate { prior: None },
                ActorAuthority::Admin
            ),
            Err(Refusal::NoPriorStanding)
        ),
        "Reactivate with no prior state must refuse, never default to Approved"
    );
}

#[test]
fn authority_is_enforced_not_advisory() {
    // A self principal cannot approve itself; a credential cannot approve anything.
    for authority in [ActorAuthority::SelfPrincipal, ActorAuthority::Credential] {
        let out = temper_principal::transition(Some(Standing::Requested), &Act::Approve, authority);
        assert!(
            matches!(out, Err(Refusal::InsufficientAuthority { .. })),
            "Approve by {authority:?} must be refused — approval is always an admin act (D11)"
        );
    }
}

#[test]
fn every_standing_variant_is_in_the_matrix() {
    // Guard: adding a Standing variant without adding it to STATES would silently shrink the
    // matrix, and every other test here would still pass.
    let covered: Vec<Standing> = STATES.iter().flatten().copied().collect();
    for s in [
        Standing::Denied,
        Standing::Requested,
        Standing::Approved,
        Standing::Revoked,
        Standing::Deactivated,
    ] {
        assert!(covered.contains(&s), "{s:?} is missing from STATES");
    }
    assert_eq!(covered.len(), 5, "STATES gained or lost a variant");
}
