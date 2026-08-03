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
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[serde(rename_all = "snake_case")]
pub enum StageDisposition {
    /// Rows returned.
    Answered,
    /// Honest zero — the question was asked and nothing matched.
    Empty,
    /// Material exists; the asker's standing does not admit disclosure at this depth.
    Withheld,
    /// The act declined a well-formed question.
    Refused,
}

/// Why an act refused. A typed variant so every door renders the same value; how a door
/// TRANSPORTS it (HTTP status, MCP error code) stays a door concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
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
}

/// A refusal, distinct from a failure and from an honest empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
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
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
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
