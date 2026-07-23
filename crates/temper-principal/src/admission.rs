use crate::{refusal::Refusal, standing::Standing};

/// Proof that a principal is admitted (spec §7).
///
/// **Constructible only by [`admit`].** The single field is private and there is no public
/// constructor, no `Default`, and no `From`, so a value of this type cannot exist without a
/// standing of `Approved` having been read and parsed.
///
/// This is a genuine enforcement, and it is new. The design doc describes it as "preserving the
/// type-state guarantee `SystemAuthorized` has today" — but `SystemAuthorized` is
/// `pub struct SystemAuthorized(pub AuthenticatedProfile)` with a public field and no `impl`
/// block (temper-services/src/auth/mod.rs:263), so any crate can build one by struct literal.
/// That gap is filed separately; it is not inherited here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedPrincipal {
    /// Private. This is the whole point of the type.
    standing: Standing,
}

impl AdmittedPrincipal {
    /// The standing that admitted this principal. Always `Approved` — exposed so a caller can log
    /// or assert it without being able to forge one.
    pub fn standing(&self) -> Standing {
        self.standing
    }
}

/// The pure, per-request admission decision (spec D1).
///
/// Takes the raw column value so that parsing — and therefore spec §7 obligation 2 — happens
/// inside the machine rather than at a call site that might default.
///
/// **One argument, deliberately.** Admission reads standing and nothing else (D15 obligation 1):
/// a `Revoked` principal is refused whether or not a review is pending, and ANDing any second
/// provisional fact into this decision restores exactly the bug shape D2 forbids. A future change
/// that adds a parameter here should be rejected at review.
///
/// **See also** the linked-identity resolver in temper-services
/// (`services::slack_link_state::resolve`), which *calls* this to decide standing and then layers
/// on the vault facts `admit` deliberately refuses to know. That is a capability gate, not an
/// admission decision — so it is allowed the conjunction D2 forbids here, and it carries no arity
/// pin. The distinction is why one function can conjoin three facts while this one must not.
pub fn admit(raw_standing: Option<&str>) -> Result<AdmittedPrincipal, Refusal> {
    // Absence denies — not an error, not a default-grant (spec §7 obligation 1).
    let Some(raw) = raw_standing else {
        return Err(Refusal::NoStanding);
    };

    // An unrecognized state denies. Never a panic, never a default (spec §7 obligation 2).
    let Some(standing) = Standing::parse(raw) else {
        return Err(Refusal::UnrecognizedStanding {
            raw: raw.to_string(),
        });
    };

    // No catchall — adding a state is a compile error here (spec §7 obligation 3).
    match standing {
        Standing::Approved => Ok(AdmittedPrincipal { standing }),
        Standing::Denied => Err(Refusal::Denied),
        Standing::Requested => Err(Refusal::Requested),
        Standing::Revoked => Err(Refusal::Revoked),
        Standing::Deactivated => Err(Refusal::Deactivated),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refusal::Refusal;

    #[test]
    fn only_approved_is_admitted() {
        assert!(admit(Some("approved")).is_ok());
        for denied in ["denied", "requested", "revoked", "deactivated"] {
            assert!(
                admit(Some(denied)).is_err(),
                "{denied} must not be admitted"
            );
        }
    }

    #[test]
    fn absence_denies() {
        // Spec §7 obligation 1 — this is what makes D7's connection-profile safety structural.
        assert_eq!(admit(None), Err(Refusal::NoStanding));
    }

    #[test]
    fn an_unrecognized_state_denies_and_names_itself() {
        // Spec §7 obligation 2 — the rolling-deploy / rollback window.
        match admit(Some("quarantined")) {
            Err(Refusal::UnrecognizedStanding { raw }) => assert_eq!(raw, "quarantined"),
            other => panic!("expected UnrecognizedStanding, got {other:?}"),
        }
    }

    #[test]
    fn each_refusal_is_distinguishable_so_the_403_can_differ() {
        // §12/D12: Denied refuses with "you may request access", Requested with "your request is
        // pending". That messaging distinction is the real justification for Requested existing as
        // a state, and it only works if the two refusals are different values.
        assert_eq!(admit(Some("denied")), Err(Refusal::Denied));
        assert_eq!(admit(Some("requested")), Err(Refusal::Requested));
        assert_eq!(admit(Some("revoked")), Err(Refusal::Revoked));
        assert_eq!(admit(Some("deactivated")), Err(Refusal::Deactivated));
    }

    #[test]
    fn admit_reads_standing_and_nothing_else() {
        // D15 obligation 1, pinned as a signature test. `admit` takes one argument. If a future
        // change gives it a second — a pending-review flag, is_active, gating membership — that
        // is the conjunction-across-provisional-facts shape D2 forbids, and this test's failure
        // is the intended alarm. Do not "fix" it by updating the call; re-read D2.
        let _: fn(Option<&str>) -> Result<AdmittedPrincipal, Refusal> = admit;
    }
}
