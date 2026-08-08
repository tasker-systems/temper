//! Per-stage disclosure. Tier 1 of design §4.4 — mandatory, O(stages), never truncated.

use serde::{Deserialize, Serialize};

use super::act::ActName;
use super::disposition::StageDisposition;
use super::envelope::NarrowedBy;
use super::scalars::MetaDetail;
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

/// A per-resource meta budget that bit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct MetaTruncated {
    pub stage: StageName,
    pub retained: i64,
    pub dropped: i64,
}

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
    /// How many of the usable ids actually contributed to what came back.
    ///
    /// **NULL MEANS THIS ACT CANNOT REPORT IT. It never means zero.** Which acts can is DECLARED —
    /// [`super::act::ActDeclaration::discloses`] — so it is knowable before running the query
    /// rather than discovered here.
    ///
    /// `follow-from` is the act that cannot: its walk discards seed provenance before returning,
    /// and the obvious fallback is worse than null, because a seed never appears in its own output
    /// and the `bound` reading would print 0 of 10 on a stage that returned forty neighbours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_contributed: Option<i64>,
    /// How many of them this stage could not use at all — invisible, nonexistent, or malformed.
    ///
    /// Conflates the three on purpose — see [`super::envelope::StageResult::input_unusable`].
    /// Naming the invisible case alone would make the trace a single-probe existence oracle.
    pub input_unusable: i64,
    pub narrowed_by: Vec<NarrowedBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_truncated: Option<MetaTruncated>,
}

/// The whole composition's disclosure: an ordered per-stage record array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CompositionTrace {
    pub meta_detail: MetaDetail,
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
            relation: None,
            input_source: None,
            input_ids: 0,
            input_contributed: None,
            input_unusable: 0,
            narrowed_by: vec![],
            meta_truncated: None,
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
        t.input_contributed = Some(0);
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
    fn a_null_contribution_is_distinguishable_from_a_zero_one() {
        // THE reason this field is an Option. Null means the act cannot report it; zero means it
        // reported none. `follow-from` is the act that cannot — its walk discards seed provenance
        // — and printing 0 there, on a stage that returned forty neighbours, would be plausible,
        // quiet and false.
        let cannot = trace(
            "neighbours",
            ActName::FollowFrom,
            StageDisposition::Answered,
        );
        let mut none_did = trace(
            "narrowed",
            ActName::FindAboutWithin,
            StageDisposition::Empty,
        );
        none_did.input_contributed = Some(0);

        let a = serde_json::to_string(&cannot).unwrap();
        let b = serde_json::to_string(&none_did).unwrap();
        assert!(
            !a.contains("input_contributed"),
            "cannot-report omits it: {a}"
        );
        assert!(
            b.contains(r#""input_contributed":0"#),
            "reported-none carries it: {b}"
        );
        assert_ne!(a, b);
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
    fn a_truncated_meta_budget_is_always_disclosed() {
        // ORPHAN_LIMIT = 50 truncates with no response flag and no server log. The contract may
        // decline to carry detail; it may never do so silently.
        let mut t = trace("near", ActName::FollowFrom, StageDisposition::Answered);
        t.meta_truncated = Some(MetaTruncated {
            stage: name("near"),
            retained: 50,
            dropped: 412,
        });
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("meta_truncated"));
        assert_eq!(serde_json::from_str::<StageTrace>(&json).unwrap(), t);
    }

    #[test]
    fn a_composition_trace_is_ordered_and_carries_its_detail_level() {
        let c = CompositionTrace {
            meta_detail: MetaDetail::Surviving,
            stages: vec![],
        };
        assert_eq!(c.meta_detail, MetaDetail::Surviving);
        assert_eq!(
            serde_json::from_str::<CompositionTrace>(&serde_json::to_string(&c).unwrap()).unwrap(),
            c
        );
    }
}
