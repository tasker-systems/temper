use crate::{act::ActorAuthority, standing::Standing};
use serde::{Deserialize, Serialize};

/// Why a principal was refused, typed.
///
/// This replaces the stringly-typed enriched 403, which carried `access_mode: String` and whose
/// tests asserted a sentinel `"join_request"` that was never a real mode
/// (`temper-services/src/error.rs:299,377` — verified: the live domain is `open`/`invite_only`).
///
/// Spec §12: "Every illegal cell asserts a *reason*, not just a refusal. The point of refusing at
/// the act is that the actor learns why; a test that only checks 'not admitted' would pass on a
/// silent denial."
// The per-variant `schema(title = …)` below are NOT decoration, and a new variant needs one too.
// An internally-tagged enum becomes an anonymous `oneOf` in OpenAPI, and openapi-generator names
// anonymous branches positionally — without the titles the Ruby gem ships `RefusalOneOf` …
// `RefusalOneOf8`, names that silently renumber the day a variant is inserted rather than appended.
// The title is what pins each generated branch to its Rust variant name. Kept out of the doc
// comment on purpose: this is generator plumbing, and doc comments land in the published spec.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admission.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Refusal {
    /// No standing row. Absence denies (spec §7 obligation 1) — this is what makes D7 structural.
    #[cfg_attr(feature = "web-api", schema(title = "NoStanding"))]
    NoStanding,
    /// The column held a value this binary does not know (spec §7 obligation 2).
    #[cfg_attr(feature = "web-api", schema(title = "UnrecognizedStanding"))]
    UnrecognizedStanding { raw: String },
    /// Provisioned but never granted. The refusal says *"you may request access."*
    #[cfg_attr(feature = "web-api", schema(title = "Denied"))]
    Denied,
    /// Asked, not yet decided. The refusal says *"your request is pending."*
    #[cfg_attr(feature = "web-api", schema(title = "Requested"))]
    Requested,
    /// Was granted and lost it. A different sentence, and a different audit signal, than `Denied`.
    #[cfg_attr(feature = "web-api", schema(title = "Revoked"))]
    Revoked,
    /// The principal itself is disabled.
    #[cfg_attr(feature = "web-api", schema(title = "Deactivated"))]
    Deactivated,
    /// The act is not legal from this state (spec §6 — every unlisted cell).
    ///
    /// `act` is an owned `String` rather than `&'static str` so `Refusal` is `DeserializeOwned`: it
    /// rides the 403 wire, and a borrowed-`'static` field would make the whole enum undeserializable
    /// from a short-lived `serde_json` input. The value is still one of a fixed set of act literals.
    #[cfg_attr(feature = "web-api", schema(title = "IllegalTransition"))]
    IllegalTransition { from: Option<Standing>, act: String },
    /// The actor lacks the authority this act requires.
    #[cfg_attr(feature = "web-api", schema(title = "InsufficientAuthority"))]
    InsufficientAuthority {
        required: ActorAuthority,
        actual: ActorAuthority,
    },
    /// `Reactivate` with no recoverable prior state. The backfill's genesis pass (spec §11) exists
    /// so this is unreachable for pre-existing rows; the machine still refuses rather than guesses.
    #[cfg_attr(feature = "web-api", schema(title = "NoPriorStanding"))]
    NoPriorStanding,
}

impl Refusal {
    /// A non-empty human-facing reason for every variant. The matrix test asserts non-emptiness
    /// across the whole cell space, so a new variant cannot ship silent.
    pub fn reason(&self) -> String {
        match self {
            Refusal::NoStanding => "no standing on this instance".to_string(),
            Refusal::UnrecognizedStanding { raw } => {
                format!("standing {raw:?} is not recognized by this build")
            }
            Refusal::Denied => "access has not been granted; you may request access".to_string(),
            Refusal::Requested => "your access request is pending review".to_string(),
            Refusal::Revoked => "access was revoked; you may request a review".to_string(),
            Refusal::Deactivated => "this principal is deactivated".to_string(),
            Refusal::IllegalTransition { from, act } => match from {
                Some(s) => format!("{act} is not legal from {}", s.as_str()),
                None => format!("{act} is not legal for a principal with no standing"),
            },
            Refusal::InsufficientAuthority { required, actual } => {
                format!("this act requires {required:?} authority; caller has {actual:?}")
            }
            Refusal::NoPriorStanding => {
                "no prior standing is recorded, so reactivation has nothing to restore".to_string()
            }
        }
    }
}
