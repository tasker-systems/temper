//! The pure linked-identity resolution: gathered evidence → a mintable proof, or a typed refusal.
//!
//! ```text
//! service gathers evidence ─► resolve() decides ─► mint decrypts / surface renders
//!   (linked? vaulted? standing)   (this module)      (slack_grant_vault_service / handlers)
//! ```
//!
//! # This is a capability gate, NOT an admission decision — and the difference is why D2 does not govern it
//!
//! `resolve` follows the *shape* of [`temper_principal::admit`] — raw evidence in, a sealed
//! proof-or-typed-reason out — and it delegates the standing question to `admit` rather than
//! restating it. But it is deliberately **not** `admit`'s twin, and it must not be pinned like one.
//!
//! `admit` answers *admission*: may this principal act at all? Spec D2
//! (`2026-07-20-principal-admission-state-machine-design.md`) forbids ANDing a second provisional
//! fact into *that* decision — `admit_reads_standing_and_nothing_else` (`admission.rs`) is the alarm
//! for exactly that, and its comment says "do not fix it by updating the call; re-read D2."
//!
//! `resolve` answers a *capability* question layered on top of an admitted human: given that they
//! are admitted, is there a credential to present as them? A mint genuinely requires all three facts
//! — a link, a vaulted grant, and admission — and requiring fewer would mint without a grant. So the
//! three-fact conjunction here is correct, and there is **deliberately no arity pin on `resolve`**:
//! copying `admit`'s anti-conjunction test onto a decision that is *supposed* to conjoin would not
//! honour the obligation, it would disable the alarm by making a reader think it was respected here.
//! `admit`'s pin stays where it is and keeps meaning what it says.
//!
//! This is instance two of the "decide server-side, return a proof-or-typed-reason" shape (`admit`
//! is instance one). Nothing is extracted into shared machinery: the repo's own convention names a
//! pattern only at three instances (see the `ScopedAuthority` design), and `ScopedAuthority` itself
//! does not fit here — it presumes a caller axis this gate lacks (the mint acts for a Slack
//! principal, with no authenticated caller profile), its `denial()` is static and cannot carry a
//! cause, and it renders denial as an `ApiError` where this surface returns a 200 with a payload.

use temper_core::types::slack::LinkRefusal;
use temper_principal::{admit, Standing};

/// The three facts a mint decision needs, gathered by the service from one query
/// (`slack_grant_vault_service`). Internal — never crosses to a surface, so it carries no
/// wire derives. `standing` is the raw column text, handed to `admit` unparsed exactly as
/// `admit` expects (`admission.rs`), so parsing — and the refusal for an unrecognized value —
/// happens inside the machine.
pub struct LinkEvidence<'a> {
    pub linked: bool,
    pub vaulted: bool,
    pub standing: Option<&'a str>,
}

/// Sealed proof that a linked identity may mint. Constructible only by [`resolve`] (private field,
/// no `Default`, no `From`), and it **never leaves the service** — it gates the decrypt/refresh
/// inside `mint_access_token` and is discarded. Mirrors `AdmittedPrincipal`'s shape one layer up.
pub struct Mintable {
    standing: Standing,
}

impl Mintable {
    /// The standing that admitted this mint — always `Approved`. Exposed for logging/assertion,
    /// not forgeable.
    pub fn standing(&self) -> Standing {
        self.standing
    }
}

/// Resolve gathered evidence into a mintable proof, or the typed reason it was refused.
///
/// **The order is forced, not chosen.** `NotLinked` first, by data availability: standing is a
/// property of the temper *profile*, so it is unknowable before a link exists. Then standing before
/// vault, by usefulness: a non-approved human must be told the remedy that works (ask an admin),
/// not the one that cannot (re-link) — that reordering is the whole fix. See the module doc for why
/// this three-fact conjunction is correct and unpinned.
pub fn resolve(ev: LinkEvidence<'_>) -> Result<Mintable, LinkRefusal> {
    if !ev.linked {
        return Err(LinkRefusal::NotLinked);
    }

    // Standing before vault. `admit` owns the standing → refusal mapping (including `NoStanding`
    // for a missing row and `UnrecognizedStanding` for a value this build does not know); we carry
    // its `Refusal` verbatim rather than restating any of it.
    let admitted = admit(ev.standing).map_err(|refusal| LinkRefusal::Standing { refusal })?;

    if !ev.vaulted {
        return Err(LinkRefusal::NotVaulted);
    }

    Ok(Mintable {
        standing: admitted.standing(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_principal::Refusal;

    /// A compile-time exhaustiveness guard: adding a `Standing` variant makes this non-exhaustive
    /// match fail to compile, which forces `standings_under_test` below to be extended too — the
    /// same trick `temper-principal/tests/matrix.rs` uses so a new state cannot slip the matrix.
    #[allow(dead_code)]
    fn _every_standing_is_accounted_for(s: Standing) {
        match s {
            Standing::Approved
            | Standing::Denied
            | Standing::Requested
            | Standing::Revoked
            | Standing::Deactivated => {}
        }
    }

    /// Every standing input the matrix exercises: absence, an unrecognized value (the rolling-deploy
    /// window), and each of the five known states by its real DB spelling.
    fn standings_under_test() -> Vec<Option<String>> {
        let known = [
            Standing::Approved,
            Standing::Denied,
            Standing::Requested,
            Standing::Revoked,
            Standing::Deactivated,
        ];
        let mut v: Vec<Option<String>> = vec![None, Some("quarantined".to_string())];
        v.extend(known.iter().map(|s| Some(s.as_str().to_string())));
        v
    }

    #[test]
    fn every_cell_is_decided_and_the_standing_arm_matches_admit_exactly() {
        for linked in [false, true] {
            for vaulted in [false, true] {
                for st in standings_under_test() {
                    let got = resolve(LinkEvidence {
                        linked,
                        vaulted,
                        standing: st.as_deref(),
                    });

                    if !linked {
                        assert_eq!(
                            got.err(),
                            Some(LinkRefusal::NotLinked),
                            "unlinked must refuse NotLinked regardless of vaulted={vaulted} standing={st:?}"
                        );
                        continue;
                    }

                    match admit(st.as_deref()) {
                        // Standing refuses → the Standing arm wins over vault state (the fix), and
                        // it carries the SAME refusal admit returns — compared directly, not restated.
                        Err(refusal) => assert_eq!(
                            got.err(),
                            Some(LinkRefusal::Standing { refusal }),
                            "a standing refusal must win over vault state (vaulted={vaulted} standing={st:?})"
                        ),
                        // Approved: vault state decides.
                        Ok(_) => {
                            if vaulted {
                                assert!(
                                    got.is_ok(),
                                    "linked + approved + vaulted must mint (standing={st:?})"
                                );
                            } else {
                                assert_eq!(
                                    got.err(),
                                    Some(LinkRefusal::NotVaulted),
                                    "linked + approved + unvaulted is NotVaulted (standing={st:?})"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn only_admit_reachable_refusals_ever_surface() {
        // `resolve` delegates standing to `admit`, so the transition-machine refusals
        // (IllegalTransition / InsufficientAuthority / NoPriorStanding) must never appear here. A
        // future change that reached for one would fail this — no catch-all hides it.
        for st in standings_under_test() {
            if let Err(LinkRefusal::Standing { refusal }) = resolve(LinkEvidence {
                linked: true,
                vaulted: false,
                standing: st.as_deref(),
            }) {
                match refusal {
                    Refusal::NoStanding
                    | Refusal::UnrecognizedStanding { .. }
                    | Refusal::Denied
                    | Refusal::Requested
                    | Refusal::Revoked
                    | Refusal::Deactivated => {}
                    other => panic!("resolve surfaced a non-admit refusal: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn refusals_round_trip_without_key_collision() {
        // This is the test whose absence would have shipped Revision 1's blocker: a newtype
        // `Standing(Refusal)` under a `kind` tag emits a duplicate `kind` key and fails HERE.
        for r in [
            LinkRefusal::NotLinked,
            LinkRefusal::NotVaulted,
            LinkRefusal::Standing {
                refusal: Refusal::Denied,
            },
            LinkRefusal::Standing {
                refusal: Refusal::UnrecognizedStanding { raw: "x".into() },
            },
        ] {
            let json = serde_json::to_string(&r).expect("serialize");
            let back: LinkRefusal = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("round-trip failed for {json}: {e}"));
            assert_eq!(back, r, "round-trip changed the value: {json}");
        }

        // Pin the nesting shape so a regression to a colliding tag is caught explicitly.
        let json = serde_json::to_string(&LinkRefusal::Standing {
            refusal: Refusal::Denied,
        })
        .unwrap();
        assert!(
            json.contains(r#""reason":"standing""#),
            "wrong outer tag: {json}"
        );
        assert!(
            json.contains(r#""refusal":{"kind":"denied"}"#),
            "standing refusal must nest under `refusal`, not flatten: {json}"
        );
    }

    #[test]
    fn every_refusal_has_a_nonempty_reason() {
        for r in [
            LinkRefusal::NotLinked,
            LinkRefusal::NotVaulted,
            LinkRefusal::Standing {
                refusal: Refusal::NoStanding,
            },
            LinkRefusal::Standing {
                refusal: Refusal::Denied,
            },
        ] {
            assert!(!r.reason().is_empty(), "{r:?} has an empty reason");
        }
    }
}
