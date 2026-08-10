//! Per-stage disclosure. Tier 1 of design §4.4 — mandatory, O(stages), never truncated.

use serde::{Deserialize, Serialize};

use super::act::ActName;
use super::disposition::{ActRefusal, StageDisposition};
use super::envelope::NarrowedBy;
use super::stage::{StageName, StageRelation};

/// Where a stage's input set came from: an earlier stage, or the caller.
///
/// `[renamed from BoundsSource — 2026-08-08]` It reports where an input came from, which is
/// direction-neutral, and the surrounding fields no longer say "bounds" — an input may be a bound
/// or a seed, and half the compositions this surface exists for alternate between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "source")]
pub enum InputSource {
    /// Verbatim from an earlier stage's `produced` set.
    ///
    /// **Keyed by the caller's own stage NAME, not an ordinal.** `[amended — 2026-08-08]` The
    /// request names stages throughout — `ActInvocation.name`, `StageInput.stage`,
    /// `ReturnSpec.stage` are all `StageName` — while this side keyed them by position, so a
    /// caller who declared `"hits"` got disclosure back saying `2`. `composition-is-legible` is a
    /// clause about what the ASKER can read, and an ordinal is not their vocabulary.
    Upstream { stage: StageName },
    /// **Reserved and currently unreachable.** No compiled plan emits this — there is no
    /// expression language (spec §9.1), so no caller-supplied expression ever sub-selects a stage's
    /// bounds. Beat C adds a test asserting no compiled plan ever produces it.
    ///
    /// It is KEPT rather than removed because [`InputSource`] is a *closed* tagged enum: deleting
    /// a variant now would make re-adding it a breaking change, whereas an unreachable-but-declared
    /// variant costs nothing. The paired removal on the refusal side went the other way — a jaq
    /// `ExpressionNotPushdownable` reason was *removed*, because `RefusalReason` is OPEN, so keeping
    /// an unraisable reason there would be a claim with no referent. The two are treated differently
    /// on the real distinction between an open and a closed vocabulary (spec §9.1).
    Expression,
    /// Supplied directly by the caller.
    Caller,
}

// `MetaTruncated` used to live here. Removed with `meta_detail` by ADJ-4 `[2026-08-10, Pete]` —
// it existed only to serve the metadata-budget concept, whose job nobody could state (YAGNI). If a
// metadata budget ever materializes it returns designed, additively.

/// One stage's mandatory disclosure. Exists whether or not the stage produced a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct StageTrace {
    pub stage: StageName,
    pub act: ActName,
    pub disposition: StageDisposition,
    /// Present iff `disposition` is `refused` — the reason and standing-aware detail. The trace
    /// covers every stage while results cover only returned ones, so this is the ONLY refusal
    /// record for an intermediate stage. **The pair rule**: identical to
    /// [`super::envelope::StageResult::refusal`], for the same reason as the input numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<ActRefusal>,
    /// Whether this stage NARROWED or REACHED, echoed back.
    ///
    /// A reader of the trace can then tell without knowing the act vocabulary — *"did stage 3
    /// narrow or expand?"* is the question `composition-is-legible` most obviously owes an answer
    /// to, and a caller reading only the response has no other way to get it. Absent for a stage
    /// with no input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<StageRelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_source: Option<InputSource>,
    /// How many ids this stage was handed. Zero for a stage with no input.
    pub input_ids: i64,
    // `input_contributed` used to sit here — removed by ratification ⟨6⟩/9d `[2026-08-09, Pete]`,
    // see the tombstone on [`super::envelope::StageResult`].
    /// How many of them this stage could not use at all — invisible, nonexistent, or malformed.
    ///
    /// Conflates the three on purpose — see [`super::envelope::StageResult::input_unusable`].
    /// Naming the invisible case alone would make the trace a single-probe existence oracle.
    pub input_unusable: i64,
    pub narrowed_by: Vec<NarrowedBy>,
}

/// The whole composition's disclosure: an ordered per-stage record array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CompositionTrace {
    // `meta_detail` used to sit here, echoed from the request. Removed with the request field by
    // ADJ-4 `[2026-08-10, Pete]` — the metadata-budget concept it served had a job nobody could
    // state, and nothing ever honoured it.
    pub stages: Vec<StageTrace>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::disposition::StageDisposition;

    fn name(s: &str) -> StageName {
        StageName::parse(s).unwrap()
    }

    fn trace(stage: &str, act: ActName, disposition: StageDisposition) -> StageTrace {
        StageTrace {
            stage: name(stage),
            act,
            disposition,
            refusal: None,
            relation: None,
            input_source: None,
            input_ids: 0,
            input_unusable: 0,
            narrowed_by: vec![],
        }
    }

    #[test]
    fn a_refused_stage_still_has_a_trace_entry() {
        // The reason disclosure lives in the envelope rather than in each act's response: a
        // refused stage has no result to attach disclosure to.
        let mut t = trace(
            "narrowed",
            ActName::FindAboutWithin,
            StageDisposition::Refused,
        );
        t.relation = Some(StageRelation::Bound);
        t.input_source = Some(InputSource::Upstream {
            stage: name("hits"),
        });
        t.input_ids = 40;
        assert_eq!(t.disposition, StageDisposition::Refused);
        assert_eq!(
            serde_json::from_str::<StageTrace>(&serde_json::to_string(&t).unwrap()).unwrap(),
            t
        );
    }

    #[test]
    fn a_trace_names_the_stage_the_caller_named_rather_than_its_position() {
        // `composition-is-legible` is a clause about what the ASKER can read. The request names
        // stages throughout; this side keyed them by ordinal, so a caller who declared "hits" got
        // disclosure back saying `2`. Both the entry and the upstream reference use the name now.
        let mut t = trace(
            "neighbours",
            ActName::FollowFrom,
            StageDisposition::Answered,
        );
        t.input_source = Some(InputSource::Upstream {
            stage: name("seeds"),
        });
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains(r#""stage":"neighbours""#), "got: {json}");
        assert!(json.contains(r#""stage":"seeds""#), "got: {json}");
        assert!(
            !json.contains(r#""stage":2"#) && !json.contains(r#""stage":1"#),
            "an ordinal is not the caller's vocabulary; got: {json}"
        );
    }

    #[test]
    fn the_trace_echoes_whether_a_stage_narrowed_or_reached() {
        // "Did stage 3 narrow or expand?" is the question a reader of the trace most obviously
        // needs answered, and without this they would have to know that `follow-from` is a
        // traversal to work it out. Absent for a stage with no input, which is not the same as a
        // stage that narrowed.
        let mut seeded = trace(
            "neighbours",
            ActName::FollowFrom,
            StageDisposition::Answered,
        );
        seeded.relation = Some(StageRelation::Seed);
        assert!(serde_json::to_string(&seeded)
            .unwrap()
            .contains(r#""relation":"seed""#));

        let rooted = trace("hits", ActName::FindExact, StageDisposition::Answered);
        assert!(!serde_json::to_string(&rooted).unwrap().contains("relation"));
    }

    #[test]
    fn a_refused_trace_entry_carries_its_reason_and_an_answered_one_omits_the_key() {
        // The pair rule's trace half (ADJ-3, 2026-08-10). The trace covers every stage while
        // results cover only returned ones, so this is the ONLY refusal record for an intermediate
        // stage — a `refused` entry with the reason stripped would leave the reader knowing a stage
        // declined and nothing about why.
        use crate::types::query::disposition::{ActRefusal, RefusalReason};
        let mut refused = trace(
            "wide",
            ActName::FindAboutAnywhere,
            StageDisposition::Refused,
        );
        refused.refusal = Some(ActRefusal {
            reason: RefusalReason::EmbeddingUnavailable,
            detail: "the server could not compute one".to_string(),
        });
        let json = serde_json::to_string(&refused).unwrap();
        assert!(json.contains("embedding_unavailable"), "got: {json}");
        assert_eq!(serde_json::from_str::<StageTrace>(&json).unwrap(), refused);

        let answered = trace("hits", ActName::FindExact, StageDisposition::Answered);
        let json = serde_json::to_string(&answered).unwrap();
        assert!(
            !json.contains("refusal"),
            "an answered entry has no refusal to carry: {json}"
        );
    }

    #[test]
    fn input_source_distinguishes_upstream_from_the_caller_and_reserves_expression() {
        // `Expression` is reserved and never emitted — there is no expression language, so nothing
        // can produce it. It is KEPT because this is a CLOSED tagged enum: deleting a variant now
        // would make re-adding it a breaking change. The paired removal on the refusal side went
        // the other way (`RefusalReason` is OPEN, so an unraisable reason there is a claim with no
        // referent). The two differ on the real distinction between an open and a closed vocabulary.
        let up = InputSource::Upstream {
            stage: name("hits"),
        };
        let ex = InputSource::Expression;
        let ca = InputSource::Caller;
        for pair in [(&up, &ex), (&ex, &ca)] {
            assert_ne!(
                serde_json::to_string(pair.0).unwrap(),
                serde_json::to_string(pair.1).unwrap()
            );
        }
        for v in [up, ex, ca] {
            assert_eq!(
                serde_json::from_str::<InputSource>(&serde_json::to_string(&v).unwrap()).unwrap(),
                v
            );
        }
    }

    #[test]
    fn a_composition_trace_round_trips() {
        // It used to also carry a `meta_detail` level; that went with the metadata-budget concept
        // (ADJ-4, 2026-08-10). What remains is the ordered per-stage record array.
        let c = CompositionTrace {
            stages: vec![trace(
                "hits",
                ActName::FindExact,
                StageDisposition::Answered,
            )],
        };
        assert_eq!(
            serde_json::from_str::<CompositionTrace>(&serde_json::to_string(&c).unwrap()).unwrap(),
            c
        );
    }
}
