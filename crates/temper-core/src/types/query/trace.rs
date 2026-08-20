//! Per-stage disclosure. Tier 1 of design §4.4 — mandatory, O(stages), never truncated.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::act::ActName;
use super::disposition::{ActRefusal, StageDisposition};
use super::envelope::NarrowedBy;
use super::hits::RegionDisclosure;
use super::scalars::{BoundTerm, Extent};
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

/// One set a stage was handed: what it was FOR, where it came from, and how big it was.
///
/// `[added — 2026-08-14]` with the widening of `ActInvocation::inputs`. It carries no `unusable`
/// count of its own — that stays one conflated number on the stage
/// ([`StageTrace::input_unusable`]), because splitting it per input would narrow what a caller can
/// probe with, and the whole reason the stage-level figure conflates invisible/nonexistent/malformed
/// is to keep it from being a single-probe existence oracle. A per-input split hands that back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct StageInputTrace {
    /// Whether this set NARROWED the stage or was REACHED from.
    pub relation: StageRelation,
    pub source: InputSource,
    /// How many ids this particular set held.
    pub ids: i64,
}

/// One stage's mandatory disclosure. Exists whether or not the stage produced a result.
///
/// **`Eq` was dropped when `disclosed_regions` arrived** `[2026-08-20]`. `region_score` is an `f64`
/// carried raw and deliberately un-normalized, so this type is `PartialEq` and cannot be `Eq`.
/// Nothing used it as a set or map key. [`super::envelope::StageResult`] never derived either.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// What this stage was handed, one entry per input.
    ///
    /// A reader can then tell without knowing the act vocabulary — *"did stage 3 narrow or
    /// expand?"* is the question `composition-is-legible` most obviously owes an answer to, and a
    /// caller reading only the response has no other way to get it. Empty for a stage with no
    /// input.
    ///
    /// # This replaced a `relation` / `input_source` PAIR, and the pair could not be kept
    ///
    /// `[widened — 2026-08-14]` with `ActInvocation::inputs`. Both were `Option`s describing *the*
    /// input, which was a total description while a stage had one. A bounded walk has two — a seed
    /// and a bound — and filling a single `relation` from whichever arrived first would answer
    /// *"did this stage narrow or expand?"* with **half the truth and no marker saying so**. That is
    /// worse than not answering: the field's whole job is to be the thing a caller trusts instead of
    /// knowing the act vocabulary.
    ///
    /// Keeping the two fields beside this list was the alternative and is the drift-by-construction
    /// this contract keeps removing — two spellings of one fact, free to disagree the moment a
    /// second input appears.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<StageInputTrace>,
    /// How many ids this stage was handed **in total, across every input**. Zero for a stage with
    /// no input.
    ///
    /// A sum rather than a per-input figure, because [`Self::input_unusable`] beside it is one
    /// conflated number by design and a total is the only thing the two can be compared against.
    /// The per-input split is in [`Self::inputs`].
    pub input_ids: i64,
    // `input_contributed` used to sit here — removed by ratification ⟨6⟩/9d `[2026-08-09, Pete]`,
    // see the tombstone on [`super::envelope::StageResult`].
    /// How many of them this stage could not use at all — invisible, nonexistent, or malformed.
    ///
    /// Conflates the three on purpose — see [`super::envelope::StageResult::input_unusable`].
    /// Naming the invisible case alone would make the trace a single-probe existence oracle.
    pub input_unusable: i64,
    /// How many ids this stage PRODUCED — the mirror of [`Self::input_ids`], and what lets a
    /// reader ask *"did this stage earn its place?"* rather than only *"did it find anything?"*
    ///
    /// Carried for EVERY stage, which is the point: an intermediate ships no rows, and a
    /// combinator can never be a returned stage at all, so for those this is the only account of
    /// their size there is.
    ///
    /// **It counts what the corpus yielded, not what came back.** For a returned stage it may
    /// exceed that stage's hydrated rows, because an id that stopped being visible between the two
    /// statements is dropped from the rows and still counted here. The two are different questions
    /// and only the gap between them is interesting.
    // # Why it is here and not on `StageResult`, and why the pair rule does not reach it
    //
    // `[added — 2026-08-15]` The number was already computed, shipped across the wire, read, spent
    // on one boolean (`Answered` vs `Empty`) and dropped. So the trace disclosed *whether* and
    // never *how much*. This is that number reaching the wire — plumbing, not a fetch.
    //
    // `input_ids`, `input_unusable` and `refusal` are each duplicated onto `StageResult` because
    // the trace covers every stage and the results only the returned ones, so disagreeing copies
    // would leave a reader unable to tell which was right. **This field inverts that argument**: a
    // returned stage already ships its rows, so a count beside them would be a THIRD spelling of
    // one fact, and the three would legitimately disagree by the hydration drop above.
    //
    // The workaround it replaces — recover it from a downstream stage's `input_ids` — fails on the
    // two shapes that matter. A combinator contributes no per-input entries, only a SUM, so two
    // arms collapse into one number identifying neither; and a terminal combinator has no
    // downstream. Measured on the register-coverage composition `[2026-08-15]`: seven stages, of
    // which exactly one — the returned one — had a knowable count.
    //
    // # Why HOW MANY is disclosable where the neighbouring numbers are conflated
    //
    // `[ruled — 2026-08-15, Pete]` `input_unusable` conflates invisible / nonexistent / malformed
    // because splitting it would be a single-probe existence oracle: it counts CALLER-SUPPLIED ids,
    // so a caller naming one id and reading the counter learns whether that id exists.
    // `produced_ids` is structurally the other thing — it counts ROWS THE CORPUS YIELDED, never ids
    // the caller named, so there is no id to probe with.
    //
    // And every row it counts is already inside this principal's visibility: every act's core call
    // takes the visible set as its first argument (`emit_ungated_core_call` in
    // `temper_substrate::readback::query_plan`), and a combinator projects over those CTEs, so it
    // inherits the gate. A cardinality over the caller's own visible corpus, for a question the
    // caller authored, is the class of thing `Extent` and `total` already disclose.
    //
    // The design had already ruled it: `final_select`'s doc says *"a tally carries how many, never
    // which"*, and NULLs the id, kind, quantity and `via` columns by construction so membership
    // stays the pipe's internal currency. How-many was the disclosable half from the start. And the
    // hardest case shipped AHEAD of this field — `NarrowedBy::admitted` on a `difference` is a
    // combinator's produced count, on the wire since 2026-08-15 — so generalizing it adds no new
    // class of disclosure, only the missing coverage.
    //
    // # The trap
    //
    // **Never fetch this separately.** The tally rides in the SAME statement as the hits, and
    // `final_select`'s doc says why: asking again would answer from a DIFFERENT SNAPSHOT, and *"a
    // trace that disagrees with the rows beside it is worse than no trace: it reads as disclosure
    // and is not."*
    pub produced_ids: i64,
    /// Complete / partial / indeterminate — **for every stage, not only the returned ones.** NOT a
    /// total; see [`Extent`].
    ///
    /// **The pair rule**: [`super::envelope::StageResult::extent`] carries the same value, and the
    /// two must stay identical for the same reason as [`Self::input_ids`], [`Self::input_unusable`]
    /// and [`Self::refusal`] — the trace covers every stage and the results only the returned ones,
    /// so disagreeing copies would leave a reader unable to tell which was right.
    ///
    /// **It is a fact about THIS stage's own bound.** A combinator applies none, so it reports
    /// `Complete`, meaning *"I dropped nothing"* rather than *"the corpus behind my arms is
    /// exhausted"*: an arm's `Partial` is not propagated up, because that would make one stage's
    /// extent a claim about another stage's bound. Every arm is a stage and carries its own entry
    /// here, which is how a reader sees that an `intersect` was computed against a truncated walk.
    // # Why an intermediate stage needs it, which is the whole reason it is here
    //
    // `[added — 2026-08-17]` A walk-then-narrow composition (`follow-from` → `intersect` → rank)
    // truncates at the WALK, and the walk is an intermediate: it never appears in `returned`, so
    // nothing in the response disclosed whether its page was full. The intersect is then computed
    // against a silently truncated set, and the final answer is plausible, well-formed and wrong —
    // which is worse than an error, because nothing in it asks to be checked.
    //
    // This is `produced_ids`' argument arriving one field over, and it does NOT invert the way that
    // one did: a count beside a returned stage's rows would be a third spelling of one fact,
    // whereas `Partial` is not derivable from rows a reader does not have.
    //
    // Both copies are read off the one `extent_of` DEFINITION that `temper-services`' `stage_numbers`
    // holds — the same single-definition the refusal beside it already has. Note what that does and
    // does not promise: `stage_numbers` is called once per CARRIER, so for a returned stage it
    // evaluates twice. The copies cannot disagree because that function is pure, not because it runs
    // once. A second definition that happened to agree today is the drift this contract removes; a
    // second evaluation of the same pure definition is not.
    //
    // # A `truncated_by_ceiling` flag was REFUSED
    //
    // `[decided — 2026-08-17, Pete]` It would be a second spelling of `Extent::Partial` — free to
    // disagree with it the moment either side gains a case, and silent about the third state
    // `Extent::Indeterminate` carries. Same argument that keeps the retired
    // `relation`/`input_source` pair from returning beside `inputs`.
    pub extent: Extent,
    /// The APPLIED value of every admitted term: the page this stage actually RAN with, clamped to
    /// the act's published ceiling and defaulted where the caller named nothing.
    ///
    /// **The pair rule** again — identical to [`super::envelope::StageResult::terms_applied`], from
    /// one [`super::registry::applied_terms`] DEFINITION rather than two. That function's own doc
    /// argues it from the other end: *"The one definition, and it exists because there are two
    /// consumers who must not disagree… Computed twice, they would eventually differ, and the
    /// difference would be a response claiming a page size that did not run."* It already had two
    /// consumers (the compiler binds these values, the assembler reports them); this adds a second
    /// READER of the map the assembler already holds, never a second definition of it.
    ///
    /// **"One definition" is the claim; "one evaluation" is not.** `stage_numbers` is called once
    /// per carrier, so a returned stage evaluates it twice. What forbids disagreement is that the
    /// definition is pure — the property to preserve if it is ever edited.
    ///
    /// Empty for a combinator, which admits no terms — a union runs no page of its own.
    ///
    /// **No "you were clamped" flag rides beside this, and this field does not add one.** That call
    /// is not taken here — it belongs to [`super::registry::applied_terms`], which states it:
    /// ceilings are published per act, so the applied value is the whole story, and clamping to a
    /// ceiling nobody published would be the bug rather than the silence. `temper-services`'
    /// `the_page_a_stage_reports_is_the_clamped_one_the_statement_actually_ran` pins the other half
    /// — *"reporting the request back would make `terms_applied` an echo rather than a
    /// disclosure"* — so what rides here is what ran, never what was asked for.
    pub terms_applied: BTreeMap<BoundTerm, i64>,
    pub narrowed_by: Vec<NarrowedBy>,
    /// Which regions a `survey` stage matched, and at what score. Empty for every other act.
    ///
    /// **This is what `discloses: [Disclosure::Region]` MEANS**, and until 2026-08-20 it meant
    /// nothing: the declaration existed in the registry with no consumer anywhere, no `region_id`
    /// in the assembler, and no field here — so a caller was told which regions matched by nothing
    /// at all. A declaration describes the DEPLOYED system; this field is what makes that true for
    /// `survey`.
    ///
    /// **The pair rule**: [`super::envelope::StageResult::disclosed_regions`] carries the same
    /// value, for the same reason as [`Self::extent`], [`Self::terms_applied`] and the input
    /// numbers — the trace covers every stage and the results only the returned ones, so
    /// disagreeing copies would leave a reader unable to tell which was right. One definition
    /// (`disclosed_regions_for`), read twice.
    #[serde(default)]
    pub disclosed_regions: Vec<RegionDisclosure>,
}

/// The whole composition's disclosure: an ordered per-stage record array.
///
/// `Eq` dropped with [`StageTrace`]'s, and for its reason — see the note there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
            inputs: vec![],
            input_ids: 0,
            input_unusable: 0,
            produced_ids: 0,
            // A stage that ran and matched nothing IS complete — an honest zero is a complete
            // answer, and the helper's default stage is one of those.
            extent: Extent::Complete,
            terms_applied: BTreeMap::new(),
            narrowed_by: vec![],
            disclosed_regions: vec![],
        }
    }

    #[test]
    fn a_disclosed_region_carries_its_score_raw_and_unclamped() {
        // `region_score` spans [-0.57, 1.05]. A carrier that clamped into [0,1] would silently
        // settle the OPEN blend ruling [2026-08-14] it is not entitled to settle — and would do it
        // invisibly, because a clamped number looks exactly like a well-behaved score. Both ends
        // are probed: one outside the interval in each direction.
        for score in [-0.57_f64, 1.05_f64] {
            let d = RegionDisclosure {
                region_id: uuid::Uuid::nil(),
                region_score: score,
            };
            let j = serde_json::to_value(&d).unwrap();
            assert_eq!(
                j["region_score"].as_f64().unwrap(),
                score,
                "serialization must not reshape the blend"
            );
            let back: RegionDisclosure = serde_json::from_value(j).unwrap();
            assert_eq!(back.region_score, score);
            assert_eq!(back.region_id, uuid::Uuid::nil());
        }
    }

    #[test]
    fn a_trace_discloses_no_regions_until_a_survey_puts_some_there() {
        // Empty rather than absent: `[]` and a missing key would both mean "no regions", and a
        // third spelling of one fact is what `ResourceHit::via` records refusing.
        assert!(trace("s", ActName::FollowFrom, StageDisposition::Answered)
            .disclosed_regions
            .is_empty());
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
        t.inputs = vec![StageInputTrace {
            relation: StageRelation::Bound,
            source: InputSource::Upstream {
                stage: name("hits"),
            },
            ids: 40,
        }];
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
        t.inputs = vec![StageInputTrace {
            relation: StageRelation::Seed,
            source: InputSource::Upstream {
                stage: name("seeds"),
            },
            ids: 3,
        }];
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
        seeded.inputs = vec![StageInputTrace {
            relation: StageRelation::Seed,
            source: InputSource::Caller,
            ids: 2,
        }];
        assert!(serde_json::to_string(&seeded)
            .unwrap()
            .contains(r#""relation":"seed""#));

        let rooted = trace("hits", ActName::FindExact, StageDisposition::Answered);
        assert!(!serde_json::to_string(&rooted).unwrap().contains("relation"));
    }

    /// **The case the singular pair could not report**, and the reason it was replaced rather than
    /// kept beside the list `[2026-08-14]`.
    ///
    /// A bounded walk carries a seed AND a bound. With one `relation` field, whichever input was
    /// written first became the stage's whole answer to *"did you narrow or expand?"* — half the
    /// truth, with nothing marking it as half.
    #[test]
    fn a_stage_with_two_inputs_discloses_both_relations_and_the_total() {
        let mut t = trace(
            "neighbours",
            ActName::FollowFrom,
            StageDisposition::Answered,
        );
        t.inputs = vec![
            StageInputTrace {
                relation: StageRelation::Seed,
                source: InputSource::Upstream {
                    stage: name("seeds"),
                },
                ids: 3,
            },
            StageInputTrace {
                relation: StageRelation::Bound,
                source: InputSource::Caller,
                ids: 40,
            },
        ];
        t.input_ids = 43;

        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains(r#""relation":"seed""#), "got: {json}");
        assert!(json.contains(r#""relation":"bound""#), "got: {json}");
        assert!(
            json.contains(r#""input_ids":43"#),
            "the stage-level count is the TOTAL across inputs, which is what `input_unusable` \
             beside it can be compared against; got: {json}"
        );
        assert_eq!(serde_json::from_str::<StageTrace>(&json).unwrap(), t);
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

    /// **A trace entry says which page it ran and whether that page was full**, on the wire.
    ///
    /// `[added — 2026-08-17]` An intermediate stage never appears in `returned`, so before these two
    /// fields a truncated walk feeding an `intersect` disclosed neither the page size it ran with nor
    /// that it had been cut off. Asserted on the SERIALIZED form rather than on the struct, because
    /// a field that fails to serialize is invisible to structural equality and this disclosure is
    /// worth exactly what a client can read.
    #[test]
    fn a_trace_entry_discloses_the_page_it_ran_and_whether_that_page_was_truncated() {
        let mut t = trace(
            "neighbours",
            ActName::FollowFrom,
            StageDisposition::Answered,
        );
        t.produced_ids = 50;
        t.terms_applied = BTreeMap::from([(BoundTerm::Limit, 50)]);
        t.extent = Extent::Partial;

        // **Navigate the document; do not grep it.** `[strengthened — 2026-08-17]` These were
        // `json.contains(r#""extent":"partial""#)` and `json.contains(r#""limit":50"#)`. `Extent`
        // is `#[serde(tag = "extent")]`, so the field serializes as `"extent":{"extent":"partial"}`
        // and the substring matched the INNER tag — it would have passed with the field renamed,
        // nested elsewhere, or moved to another struct in the same document. The stated purpose,
        // catching a field that fails to serialize, needs the path from the root.
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(
            v["terms_applied"]["limit"], 50,
            "the page this stage RAN with rides on the trace, keyed by term: {v}"
        );
        assert_eq!(
            v["extent"]["extent"], "partial",
            "`Extent` is the incumbent name for \"was I truncated\", and it is on the TRACE — the \
             carrier that covers intermediate stages, where a truncation is otherwise invisible: {v}"
        );
        let json = serde_json::to_string(&t).unwrap();
        assert!(
            !json.contains("truncated_by"),
            "no second spelling of `partial`: {json}"
        );
        assert_eq!(serde_json::from_str::<StageTrace>(&json).unwrap(), t);
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
