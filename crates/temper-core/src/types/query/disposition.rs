//! How a stage resolved, and what a refusal says.
//!
//! Both public types here are named away from an incumbent that holds the obvious name, and the
//! divergence is deliberate (design §5.1). [`StageDisposition`] is not
//! [`crate::types::invocation::Disposition`], which is an *invocation*'s terminal outcome
//! (`completed`/`failed`/`abandoned`). [`ActRefusal`] is not `temper_principal::Refusal`, which is
//! the *admission* refusal riding the 403 — this type inherits that doctrine's disclosure-depth
//! rule without replacing the type, since an act declining a well-formed question is a different
//! event from a principal being denied admission. Whether the two should eventually compose is
//! recorded as open in design §5.1, not settled here.

use serde::{Deserialize, Serialize};

/// How a single stage resolved. CLOSED — adding a variant is a breaking change (design §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum StageDisposition {
    /// Rows returned.
    Answered,
    /// Honest zero — the question was asked and nothing matched.
    Empty,
    /// Material exists; the asker's standing does not admit disclosure at this depth.
    ///
    /// **Only safe where existence is ALREADY licensed** — a region the asker can read containing
    /// a node they cannot. It is a statement that something is there, so it discloses existence by
    /// construction.
    ///
    /// **Never safe for a caller-supplied id.** An id the caller named and cannot see contributes
    /// as an honest [`StageDisposition::Empty`], never as a withholding: the alternative is a
    /// single-probe existence oracle. Same doctrine as `CONTEXT_REFUSAL`'s byte-identical
    /// `NotFound` and the audit gate's denial arms — see decision
    /// `019fcd13-4e65-7213-ac6f-20c3c8ccfce1`.
    Withheld,
    /// The act declined a well-formed question.
    Refused,
}

/// Why an act refused. A typed variant so every door renders the same value; how a door
/// TRANSPORTS it (HTTP status, MCP error code) stays a door concern.
///
/// OPEN vocabulary, deliberately — design §6.1 settled openness for the `act` discriminator and
/// for `disposition` but never ruled on refusals, and v0 first shipped this closed by default.
/// Corrected by decision `019fcd13-4e65-7213-ac6f-20c3c8ccfce1`: the growth this contract wants
/// includes new ways to decline, so a closed enum would make every future reason a breaking
/// change. Contrast [`StageDisposition`], which stays closed on purpose — four dispositions,
/// matched exhaustively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// The act does not accept bounds of the supplied `IdKind`.
    UnsupportedBoundKind,
    /// The act does not accept seeds of the supplied `IdKind`.
    UnsupportedSeedKind,
    /// A region set arrived without the `provenance` its kind requires.
    MissingProvenance,
    /// The act is declared but not built (`build_state` is not `served` or `fused`).
    NotImplemented,
    /// A required input the composition never supplied — e.g. a `find-about-*` stage with no
    /// threaded intention. Explicitly NOT a silent substitution.
    MissingIntention,
    /// A filter value outside a closed vocabulary — an unknown `doc_type`, `stage` or `status`.
    /// Refused rather than returned as an empty page: a typo must never be reportable as an
    /// absence, which is what four filters do today.
    UnknownFilterValue,
    /// A filter slot the act does not admit (`ResourceFilter` on an edge-only act, or the
    /// reverse). Declined, never ignored.
    FilterNotApplicable,
    /// A bound term was supplied to an act for which that frame of reference does not exist —
    /// e.g. `limit` (rows) handed to `survey`, whose bound is a funnel width. The term is never
    /// reinterpreted to fit; it is declined. Raised STATICALLY, at plan validation against the
    /// generated schemas, so an inapplicable bound is a property of the plan rather than a
    /// runtime surprise.
    BoundTermNotApplicable,
    /// The act is declared and built, but its mechanic is not reachable from THIS surface — the
    /// compiler emits no fragment for its `served_by` function yet. Spec §7's own wording for the
    /// distinction `BuildState` cannot draw: existence versus reachability-from-this-surface.
    ///
    /// The live instance before beat D is the three `find` acts: `BuildState::Served` with their
    /// own SQL functions (`search_exact` / `search_wide`), which the beat-C compiler does not emit.
    /// `NotImplemented` would be false about them (they are served); this variant is the honest one.
    /// Beat D deletes the refusal for them by giving the compiler their fragments. Fusion is not the
    /// reason — `follow-from` and `survey` are the `Fused` declarations and are exactly the acts
    /// this surface CAN reach, so the rule keys on the callable-fragment set, never on `build_state`.
    NotSeparablyReachable,
    /// A reason this consumer does not recognize. Never constructed by this crate — only by
    /// deserializing a producer newer than this consumer.
    #[serde(untagged)]
    Other(String),
}

impl RefusalReason {
    /// Whether this is a reason the running binary knows how to interpret.
    pub fn is_known(&self) -> bool {
        !matches!(self, RefusalReason::Other(_))
    }
}

/// A refusal, distinct from a failure and from an honest empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ActRefusal {
    pub reason: RefusalReason,
    /// Human-readable, disclosed at the depth the asker's standing allows.
    pub detail: String,
}

/// What a composition does when a stage refuses. Declared BEFORE execution; the executor never
/// improvises it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RefusalDisposition {
    Halt,
    DegradeAndDisclose,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_exactly_four_dispositions() {
        let all = [
            StageDisposition::Answered,
            StageDisposition::Empty,
            StageDisposition::Withheld,
            StageDisposition::Refused,
        ];
        let rendered: Vec<String> = all
            .iter()
            .map(|d| serde_json::to_string(d).unwrap())
            .collect();
        assert_eq!(
            rendered,
            ["\"answered\"", "\"empty\"", "\"withheld\"", "\"refused\""]
        );
    }

    #[test]
    fn disposition_is_closed_unknown_values_fail_to_parse() {
        // Closedness is the property that lets consumers match exhaustively. Adding a fifth
        // variant is a BREAKING change (design §6.1) precisely because of this test.
        assert!(serde_json::from_str::<StageDisposition>("\"partially_answered\"").is_err());
    }

    #[test]
    fn empty_and_refused_are_distinguishable() {
        // An honest zero and a declined question are different answers; collapsing them is the
        // refusal-dialect divergence this contract exists to end.
        assert_ne!(
            serde_json::to_string(&StageDisposition::Empty).unwrap(),
            serde_json::to_string(&StageDisposition::Refused).unwrap()
        );
    }

    #[test]
    fn refusal_carries_a_reason_and_round_trips() {
        let r = ActRefusal {
            reason: RefusalReason::UnsupportedBoundKind,
            detail: "act `find-exact` does not accept bounds of kind `region`".to_string(),
        };
        assert_eq!(
            serde_json::from_str::<ActRefusal>(&serde_json::to_string(&r).unwrap()).unwrap(),
            r
        );
    }

    #[test]
    fn refusal_reason_is_open_so_a_new_way_to_decline_is_additive() {
        // Design §6.1 ruled on `act` (open) and `disposition` (closed) and never on refusals; v0
        // shipped this closed by default. A closed enum would make every future reason a breaking
        // change, so it is open and a known variant reports `is_known`.
        let r: RefusalReason =
            serde_json::from_str("\"quota_exhausted\"").expect("unknown reason must parse");
        assert_eq!(r, RefusalReason::Other("quota_exhausted".to_string()));
        assert!(!r.is_known());
        assert!(RefusalReason::UnsupportedBoundKind.is_known());
    }

    #[test]
    fn the_removed_expression_reason_is_no_longer_a_known_variant() {
        // There is no expression language (spec §9.1), so nothing can raise this. `RefusalReason`
        // is OPEN, so removing it now costs nothing and re-adding it later is additive. Keeping a
        // reason nothing can raise is a claim about the system with no referent.
        let r: RefusalReason = serde_json::from_str("\"expression_not_pushdownable\"").unwrap();
        assert_eq!(
            r,
            RefusalReason::Other("expression_not_pushdownable".to_string())
        );
        assert!(
            !r.is_known(),
            "an old producer's value degrades to Other, it does not fail"
        );
    }

    #[test]
    fn the_disposition_enum_stays_closed_while_refusals_open() {
        // The two openness rules are deliberately different and must not converge: a consumer
        // matches four dispositions exhaustively, but must tolerate a reason it has never seen.
        assert!(serde_json::from_str::<StageDisposition>("\"quota_exhausted\"").is_err());
        assert!(serde_json::from_str::<RefusalReason>("\"quota_exhausted\"").is_ok());
    }

    #[test]
    fn composition_refusal_disposition_has_two_v0_values() {
        assert_eq!(
            serde_json::to_string(&RefusalDisposition::Halt).unwrap(),
            "\"halt\""
        );
        assert_eq!(
            serde_json::to_string(&RefusalDisposition::DegradeAndDisclose).unwrap(),
            "\"degrade_and_disclose\""
        );
    }
}
