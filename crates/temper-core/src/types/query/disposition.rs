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
    /// A cogmap or context bound arrived carrying anything other than exactly one id.
    ///
    /// **Distinct from [`RefusalReason::UnsupportedBoundKind`] on purpose, and the distinction is
    /// the whole reason this variant exists.** The kind IS accepted — reporting "this act does not
    /// narrow on that kind of id" would teach the caller the wrong lesson (*stop sending cogmap
    /// bounds*) when the right one is *send one*. `RefusalReason` is OPEN precisely so a reason that
    /// says the true thing can be added without breaking a client.
    ///
    /// The constraint is the fragments': a resource bound is a `uuid[]`, but a cogmap or context
    /// bound is served by an `(anchor_table, anchor_id)` PAIR, which holds one id. Spec §9 names the
    /// mismatch — *"an `IdSet` holds N ids; an anchor slot holds one"* — as an open cardinality gap,
    /// and refusing is the honest response to it. Silently anchoring on the first element would
    /// answer a different question than the one asked and look like a successful narrowing.
    ///
    /// **Zero refuses too, and is not "unbounded".** For a resource array the fragments distinguish
    /// `'{}'` (bounded to nothing) from `NULL` (unbounded); an anchor has no such pair, so an empty
    /// anchor set is a caller who asked to scope and named nothing. Admitting it would drop the
    /// scope and answer the unscoped question.
    ///
    /// `[moved from the compiler — 2026-08-09, Pete]` The compiler used to raise this as an `Err`
    /// that aborted the whole composition, costing every innocent stage beside it. It is a STATIC
    /// property of the plan, so it belongs here, in the 400 with every other refusal at once.
    AnchorTakesOneId,
    /// The act does not accept seeds of the supplied `IdKind`.
    UnsupportedSeedKind,
    /// A region set arrived without the `provenance` its kind requires.
    MissingProvenance,
    /// The act is declared but not built (`build_state` is not `served` or `fused`).
    NotImplemented,
    /// A find stage with no threaded intention — no QUESTION, not no vector. Explicitly NOT a
    /// silent substitution.
    ///
    /// The distinction is load-bearing and easy to lose. This fires when the composition supplied
    /// no query text at all, which `find-exact` cannot work around: its text becomes `p_query` and
    /// there is nowhere else to source it. It does **not** fire for a missing embedding — the
    /// server computes one, because API callers structurally cannot, and a failed attempt is
    /// [`RefusalReason::EmbeddingUnavailable`] instead.
    MissingIntention,
    /// `ReturnSpec.with` named a hydration section this door does not offer.
    ///
    /// A refusal rather than a parse error on purpose. The section vocabulary is shared with
    /// `show` and `list`, so `body` and `edges` are real words that deserialize cleanly — they are
    /// simply not what `/api/query` hydrates, and saying so (with a pointer to the door that does)
    /// is worth more than a deserializer's "unknown variant". It also keeps the promise that every
    /// refusal comes back at once: serde failures short-circuit before validation runs.
    SectionNotAvailable,
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
    /// **The one refusal that is not static.** A `find-about-*` stage needed a query embedding,
    /// the server had to compute it because the caller could not, and the attempt failed — an
    /// error, a panic, or the query-embed budget expiring.
    ///
    /// It is a refusal rather than an empty because the stage holds a well-formed question it
    /// cannot answer; returning `empty` would hand back a list that reads like an answer. It is
    /// also the reason every OTHER refusal here can be reported as a 400 before anything runs:
    /// this is the only one that can appear mid-flight, and it is what gives a runtime refusal a
    /// referent at all. `[decided — 2026-08-08, Pete]`
    EmbeddingUnavailable,
    /// A reason outside the declared vocabulary.
    ///
    /// `[corrected — 2026-08-09]` This said "Never constructed by this crate — only by
    /// deserializing a producer newer than this consumer." **That is false**, and was when it was
    /// written: `validate` constructs it for twelve topology and vocabulary refusals (`"cycle"`,
    /// `"dangling-reference"`, `"duplicate-stage-name"`, `"combinator-arity"`,
    /// `"unknown-return-stage"`, `"duplicate-return-stage"`, `"combinator-not-returnable"`,
    /// `"unknown-act"`, `"empty-property-key"`, `"empty-contains"`, `"no-stages"`,
    /// `"no-returns"`). So the server emits reasons its own [`RefusalReason::is_known`] answers
    /// `false` to, and those twelve are kebab-case while every declared variant is snake_case — a
    /// client's vocabulary is two conventions. Found in review; recorded rather than repaired,
    /// because promoting twelve strings to variants is a wire change and belongs with the door,
    /// not ahead of it. `[recounted — 2026-08-10]` An earlier count here said nine while listing
    /// ten and omitting `"no-stages"`; the list is now held to
    /// `grep -o 'RefusalReason::Other("[a-z-]*"' validate.rs | sort -u`.
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

// `RefusalDisposition {halt | degrade_and_disclose}` used to live here, as
// `Composition.on_stage_refusal`. Both are gone — see the block where the field was, in
// `composition.rs`. The short version: it described a case that cannot occur, and the one runtime
// refusal that CAN occur is composition-wide rather than per-stage and has two observationally
// near-identical settings. The type comes back with the field if a later runtime refusal genuinely
// wants a choice; re-adding an optional field is additive, shipping a required one against a case
// with no referent is not. `[decided — 2026-08-08, Pete]`

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
    fn the_removed_refusal_disposition_degrades_rather_than_failing_for_an_old_producer() {
        // `RefusalDisposition` is gone with its field. Anything that used to send `"halt"` now
        // sends an ignored unknown field, and nothing here can raise the old values — this pins
        // that the removal did not leave a parse hazard behind, the same way the retired
        // `expression_not_pushdownable` reason is pinned above.
        assert!(serde_json::from_str::<RefusalReason>("\"halt\"").is_ok());
        assert!(!serde_json::from_str::<RefusalReason>("\"halt\"")
            .unwrap()
            .is_known());
    }

    #[test]
    fn exactly_one_refusal_reason_is_runtime_and_the_rest_are_static() {
        // The property the 400-carries-everything design rests on: if a second reason ever becomes
        // runtime-only, `POST /api/query` can no longer promise that a composition which passes
        // validation will not refuse mid-flight for a reason it could have named up front.
        //
        // Not derivable from the type — openness means the enum cannot enumerate itself — so this
        // is a declared list, and the assertion is that the declared runtime set has exactly one
        // member. A reader adding a runtime reason has to come here and say so.
        const RUNTIME: [RefusalReason; 1] = [RefusalReason::EmbeddingUnavailable];
        assert_eq!(RUNTIME.len(), 1);
        assert!(RUNTIME[0].is_known());
        assert_eq!(
            serde_json::to_string(&RefusalReason::EmbeddingUnavailable).unwrap(),
            "\"embedding_unavailable\""
        );
    }

    #[test]
    fn a_missing_question_and_a_failed_embedding_are_different_refusals() {
        // The contradiction this pair was written to close. One is static and about the caller's
        // request (no words to search for); the other is runtime and about the server's own
        // failure to embed on their behalf. Collapsing them would either refuse every API caller
        // who cannot precompute a vector, or report a server-side failure as a caller mistake.
        assert_ne!(
            RefusalReason::MissingIntention,
            RefusalReason::EmbeddingUnavailable
        );
        assert_eq!(
            serde_json::to_string(&RefusalReason::MissingIntention).unwrap(),
            "\"missing_intention\""
        );
    }
}
