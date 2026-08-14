//! Capability — the shape is fine, and this server has not built it yet.
//!
//! Everything here may move as beats land, which is why none of it may be raised by a client
//! against a server it does not share a binary with. Note that reading a declaration is NOT what
//! makes a check belong here: the `FilterNotApplicable` sites, the
//! [`SectionNotAvailable`](RefusalReason::SectionNotAvailable) loop, and the more-than-one
//! [`AnchorTakesOneId`](RefusalReason::AnchorTakesOneId) arm read no declaration at all and are
//! pure door capability. Only some of them say *"does not yet apply"* outright; the rest read like
//! permanent structural facts and are not. **Four sites** `[was five — 2026-08-14]`, and no two are
//! retired by the same thing:
//!
//! - property predicates — Task 10b;
//! - the seven-field resource narrowing on any act OTHER than `find-resources-with` — retired by
//!   nothing, because it is no longer a shortfall. `[amended — 2026-08-14]` It was *"the six-field
//!   resource narrowing — a compiler slot that does not exist yet"*, beside a seventh entry for
//!   *"the one-value doc-type narrowing — a fragment parameter a later fragment can widen"*. **Both
//!   descriptions are now wrong, and in opposite directions**: the slot exists (the selection
//!   fragment applies all seven), and the doc-type site is gone entirely rather than widened — the
//!   narrowing moved to an act, so on a find act it refuses like the other six. That is why the
//!   count here went from six to five while the CAPABILITY grew;
//!
//!   **and then from five to four**, when the unconditional `edge_filter` site retired
//!   `[2026-08-14]`. That one WAS a "does not yet apply" — Task 11 — and `20260814000030` plus
//!   `follow-from`'s emitter arm is what retired it: the walk applies both of `EdgeFilter`'s axes.
//!   What remains is the per-act check in [`check_act`], which refuses an edge filter on every act
//!   that does not traverse an edge; it was unreachable while the unconditional site refused
//!   everything first, so this retirement is what put it into service rather than merely deleting a
//!   check;
//! - the one-id anchor slot — the fragments' `(anchor_table, anchor_id)` pair, retired by an
//!   `anchor_ids uuid[]`;
//! - [`SectionNotAvailable`](RefusalReason::SectionNotAvailable) — a widened
//!   [`ReturnSpec::ADMITTED_SECTIONS`].
//!
//! **The pass has two halves, and only one of them is gated on the topology.** [`validate_stages`]
//! reads the stage graph; [`validate_returns`] compares `with` against a constant and reads no
//! graph at all, so [`super::validate`] runs it whatever the plan's shape.
//!
//! The converse rule, and what lives on the other side of the seam, is in [`super::shape`].

use std::collections::BTreeMap;

use crate::types::query::act::{ActName, BuildState};
use crate::types::query::composition::{Composition, ReturnSpec, StageNode};
use crate::types::query::disposition::RefusalReason;
use crate::types::query::envelope::ActInvocation;
use crate::types::query::filter::FilterField;
use crate::types::query::id_set::IdKind;
use crate::types::query::registry::declaration;
use crate::types::query::stage::{StageInput, StageName, StageRelation};
use crate::types::resource_view::ResourceSection;

use super::{act_wire_name, emitted_fragment_for, refusal, term_wire_name, PlanRefusal};

/// The most facet predicates one selection may carry.
///
/// Not a round number chosen for looks: real selections in this corpus carry one or two facets, and
/// the measured cost cliff is three orders of magnitude above this — so the cap is far above any
/// question anyone asks and far below anything that hurts. A caller who genuinely needs more is
/// asking a different question and should say so with a second stage.
const MAX_FACET_PREDICATES: usize = 32;

/// The kind an upstream node produces, walking a combinator to its first input. `None` for a
/// dangling reference (already refused as topology) or an act that produces nothing.
///
/// It reads the declarations, which is why it is here and not beside the topology it feeds: what a
/// stage produces is a declared fact, and a client cannot answer it for a server it does not share
/// a binary with.
fn produced_kind_of(name: &str, by_name: &BTreeMap<&str, &StageNode>) -> Option<IdKind> {
    match by_name.get(name)? {
        StageNode::Act(inv) => declaration(&inv.act)?.produces,
        StageNode::Combine(c) => produced_kind_of(c.inputs.first()?.as_str(), by_name),
    }
}

/// The per-stage half: what each act asks of a door that has not built it yet.
///
/// Reads the stage graph — `produced_kind_of` walks upstreams — which is why [`super::validate`]
/// runs it only over a plan that topologically sorts.
///
/// Takes the refusals by `&mut` rather than returning its own, because a caller repairing a plan
/// sees ONE list and neither pass's findings outrank the other's.
pub(super) fn validate_stages(
    c: &Composition,
    by_name: &BTreeMap<&str, &StageNode>,
    errs: &mut Vec<PlanRefusal>,
) {
    for node in &c.stages {
        if let StageNode::Act(inv) = node {
            check_act(inv, node.name(), by_name, errs);
        }
    }
}

/// The `returns` half: sections this door does not hydrate.
///
/// **Separate from [`validate_stages`] because it reads no stage graph.** It compares each entry's
/// `with` against the constant [`ReturnSpec::ADMITTED_SECTIONS`] and never looks a stage up, so
/// there is nothing about a cyclic plan that makes its answer unavailable — and dropping it for one
/// would take a refusal away from a caller who used to get it, against this module's own rule that
/// `validate` returns every refusal rather than the first.
pub(super) fn validate_returns(c: &Composition, errs: &mut Vec<PlanRefusal>) {
    for ret in &c.outcome.returns {
        // Refused here rather than at deserialization, which is the whole reason `with` carries
        // the shared `ResourceSection` vocabulary instead of a narrow query-local enum: a serde
        // failure short-circuits before this function runs, so a caller with several problems
        // would learn about one of them, phrased by a deserializer.
        for section in &ret.with {
            if !ReturnSpec::ADMITTED_SECTIONS.contains(section) {
                errs.push(refusal(
                    Some(&ret.stage),
                    RefusalReason::SectionNotAvailable,
                    format!(
                        "`{section}` is not a section this door hydrates (it offers: {}). \
                         {}",
                        ReturnSpec::ADMITTED_SECTIONS
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        section_advice(*section),
                    ),
                ));
            }
        }
    }
}

/// Where to go for a section this door declines.
///
/// A refusal that only says no leaves the caller to guess, and both of these have a real home. The
/// advice is part of the refusal rather than documentation because the caller is reading the
/// refusal, not the docs.
fn section_advice(section: ResourceSection) -> &'static str {
    match section {
        // `fill_sections` records that the body arm "is an N+1 by construction" and that what
        // keeps it honest is "the door, not the loop". This is a new door and is narrow from its
        // first line.
        ResourceSection::Body => "Bodies are read one at a time — ask `show` for the ones you want",
        ResourceSection::Edges => {
            "Edge listing has its own commands; `follow-from` with an `edge_filter` walks and \
             filters on edges without returning them"
        }
        ResourceSection::OpenMeta => "",
    }
}

/// Declaration-driven checks for one act node. Every axis is independent — a node can fail more
/// than one — so nothing short-circuits; a caller sees the whole picture.
fn check_act(
    inv: &ActInvocation,
    name: &StageName,
    by_name: &BTreeMap<&str, &StageNode>,
    errs: &mut Vec<PlanRefusal>,
) {
    let Some(decl) = declaration(&inv.act) else {
        // The shape pass has already refused `ActName::Other`. Reaching here means a KNOWN variant
        // with no declaration — an internal inconsistency, not a caller error.
        //
        // **Nothing holds that, and this `return` fails OPEN. Said plainly rather than covered,
        // because a bare `return` reads as covered.** `declaration` is a `find` over
        // `search_family()`, not an exhaustive match, and no production code matches `ActName`
        // exhaustively either; `registry.rs`'s `the_search_family_declares_seven_acts_including_
        // the_anti_act` asserts a count of seven plus a hardcoded list, all of which an EIGHTH
        // variant declaring nothing would pass unchanged. Such an act would clear both passes and
        // reach `query_plan.rs`'s `_` arm, which emits `__temper_unbound_act` — a function that
        // deliberately does not exist — so it would fault at execution where the pre-split code
        // refused it as `unknown-act` with a 400. Unreachable while the enum and the family agree.
        return;
    };

    // Build-state / reachability. Keyed on the callable-fragment set, never on the `Fused`
    // discriminant — see CALLABLE_FRAGMENTS.
    match &decl.build_state {
        BuildState::Unbuilt => errs.push(refusal(
            Some(name),
            RefusalReason::NotImplemented,
            "the act is declared but not built",
        )),
        BuildState::Served | BuildState::Fused { .. } => {
            let served = decl.served_by.as_deref().unwrap_or_default();
            if emitted_fragment_for(served).is_none() {
                errs.push(refusal(
                    Some(name),
                    RefusalReason::NotSeparablyReachable,
                    format!("act mechanic `{served}` is not reachable from this surface yet"),
                ));
            }
        }
    }

    // Input kind, read against the relation the EDGE declares.
    //
    // `[amended — 2026-08-08]` This used to read `matches!(inv.bounds_mode, Some(BoundsMode::Seed))`
    // off the invocation. That was wrong in a way no test could see: `bounds_mode` was an `Option`
    // whose "required whenever `input` is present" invariant lived in prose, so an input with no
    // relation fell through to `false` and was silently checked against `accepts_bounds`. The
    // relation now rides the input, is total, and cannot be absent.
    // **Every input, each against the list its OWN relation names.** `[widened — 2026-08-14]` A
    // stage may now carry a seed and a bound at once, so this is a loop rather than a single
    // `if let`. Checking one of them would admit the other unexamined — and the unexamined one is
    // whichever slot a caller adopts second, which is the newest and least-exercised path.
    for input in &inv.inputs {
        let incoming = match input {
            StageInput::Caller { ids, .. } => Some(ids.kind.clone()),
            StageInput::Upstream { stage, .. } => produced_kind_of(stage.as_str(), by_name),
        };
        if let Some(kind) = incoming {
            let as_seed = input.relation() == StageRelation::Seed;
            if as_seed && !decl.accepts_seeds.contains(&kind) {
                // The negative face of putting the relation on the wire: this caller asked to
                // REACH BEYOND the set, and this act can only narrow within one. Deriving the
                // relation from the act would have executed the narrowing instead — a different
                // question, answered confidently.
                errs.push(refusal(
                    Some(name),
                    RefusalReason::UnsupportedSeedKind,
                    format!(
                        "act `{}` cannot grow from a set — it does not accept seeds of kind \
                         `{kind:?}`. Narrowing within the set instead would answer a different \
                         question than the one asked",
                        act_wire_name(&inv.act)
                    ),
                ));
            } else if !as_seed && !decl.accepts_bounds.contains(&kind) {
                errs.push(refusal(
                    Some(name),
                    RefusalReason::UnsupportedBoundKind,
                    format!(
                        "act `{}` does not accept bounds of kind `{kind:?}`",
                        act_wire_name(&inv.act)
                    ),
                ));
            }
        }
    }

    // The anchor slot's CARDINALITY, above one.
    //
    // `[split from the shape pass — 2026-08-12, Pete]` The zero case stays there: naming nothing to
    // scope to is malformed whatever the door. This one is not. `Cogmap` and `Context` bounds are
    // served by an `(anchor_table, anchor_id)` PAIR, and a pair holds one id — a fragment
    // parameter's shape — the class `f.doc_type.len() > 1` used to belong to before that check was
    // retired on 2026-08-14 (the narrowing moved to an act, so the arity limit went with it) — and
    // exactly what an
    // `anchor_ids uuid[]` would retire. Held in the shape pass it would let a client refuse a plan
    // a widened server runs.
    //
    // Read off the CALLER's set only, which is the same restriction the shape half carries: what an
    // upstream stage produces is a declared fact, and the anchor kinds are accepted but never
    // produced, so an upstream set is never anchor-kind.
    for ids in inv.inputs.iter().filter_map(|i| match i {
        StageInput::Caller { ids, .. } => Some(ids),
        StageInput::Upstream { .. } => None,
    }) {
        if matches!(ids.kind, IdKind::Cogmap | IdKind::Context) && ids.ids.len() > 1 {
            errs.push(refusal(
                Some(name),
                RefusalReason::AnchorTakesOneId,
                format!(
                    "a `{:?}` bound is served by the anchor pair, which holds exactly one id; \
                     this stage supplied {}. Anchoring on one of them would answer a different \
                     question than the one asked",
                    ids.kind,
                    ids.ids.len()
                ),
            ));
        }
    }

    // ── NARROWINGS THIS DOOR DECLARES BUT DOES NOT APPLY ────────────────────────────────────────
    //
    // `[added — 2026-08-09]` **Closing a silent question substitution, found in review.** Every
    // slot below was accepted by validation and then IGNORED by the compiler: `emit_ungated_core_call`
    // passes literal `NULL` in the fragment's one filter parameter and has no slot at all for the
    // rest. So a caller asking for sessions about X received anything about X — their question
    // answered as a different question, confidently, with a full page of plausible rows.
    //
    // Worse, the response then ECHOED the filter back in `narrowed_by` as though it had been
    // applied, which is the evidence a caller would use to believe the answer.
    //
    // Refusing is the contract's own rule: a narrowing is "declined, never ignored". The reason is
    // `FilterNotApplicable`, whose name is imperfect here — the ACT admits these; it is this DOOR
    // that cannot apply them — so each detail says so explicitly. Whether that deserves its own
    // reason is an open question, not a silent choice.
    // `[amended — 2026-08-14]` **The resource filter is now a PARAMETER OF ONE ACT, and every other
    // act refuses all seven of its fields — `doc_type` included.**
    //
    // This block used to refuse six and let `doc_type` through, because `doc_type` was the one field
    // the find fragments could apply. `find-resources-with` now applies all seven against real
    // storage (`query_find_resources_with`, migration `20260814000010`), so the narrowing has an act
    // and a modifier saying the same thing would be a second spelling — the drift-by-construction
    // this contract keeps removing.
    //
    // **`doc_type`'s retirement REFUSES rather than being ignored, and that inversion is
    // deliberate** `[decided — 2026-08-14, Pete]`. The established degradation for a retired field
    // is accepted-and-ignored (`a_composition_that_still_carries_bounds_or_meta_detail_is_accepted_
    // and_both_ignored`), and that is correct for a field that never did anything and WRONG here:
    // `doc_type` works today, so a caller whose narrowing silently stopped applying would get more
    // rows and no signal — precisely the substitution this act exists to remove. The detail names
    // the replacement, because a refusal that does not say where the capability went is a dead end.
    //
    // Sequencing was part of the work: this fires in the same change that gives the act its
    // mechanic, never before it.
    if let Some(f) = &inv.resource_filter {
        if matches!(inv.act, ActName::FindResourcesWith) {
            // The act whose parameter this is. Its fragment has a slot for every field, `doc_type`
            // takes a set rather than one value, and there is nothing here to refuse — except the
            // one field whose LENGTH is a cost multiplier rather than a narrowing.
            //
            // **`facets` is the only per-ROW multiplier on this surface.** `[added — 2026-08-14,
            // found in adversarial review]` Its fragment predicate is a nested `NOT EXISTS` over
            // this array, evaluated once per candidate row and short-circuiting on the first
            // predicate that fails — so cost is `|visible set| × |facets|`, and an authenticated
            // caller chooses the second factor. `tags` and `doc_type` do NOT have this shape:
            // array containment and `= ANY` are single operations whatever their length.
            //
            // Refused rather than clamped, because clamping would silently answer a different
            // question — the same reason a multi-value `doc_type` was refused rather than truncated
            // to its first element while that slot held one value.
            //
            // **This closes the instance and not the class.** Nothing bounds a composition's stage
            // count, no `statement_timeout` exists anywhere in the repo, and `acquire_timeout`
            // bounds waiting for a connection rather than execution — so a caller can still spend
            // arbitrary time by other means. That is task
            // `01a000ee-9fec-7283-baa5-75cd1580f023`, which is not fixed here
            // `[decided — 2026-08-14, Pete]`: a global execution bound is a decision about every
            // query on the surface rather than about this act, and it may want a read/write path
            // split rather than a single setting — so it gets a focused session, not a corner of
            // this one.
            if f.facets.len() > MAX_FACET_PREDICATES {
                errs.push(refusal(
                    Some(name),
                    RefusalReason::FilterNotApplicable,
                    format!(
                        "a selection admits at most {MAX_FACET_PREDICATES} facet predicates; this                          stage supplied {}. Each one is evaluated against every candidate row, so                          the list is a cost multiplier rather than a narrowing — narrow with the                          other fields first, or split the question",
                        f.facets.len()
                    ),
                ));
            }
        } else {
            let declared: &[(&str, bool)] = &[
                ("doc_type", !f.doc_type.is_empty()),
                ("tags", !f.tags.is_empty()),
                ("facets", !f.facets.is_empty()),
                ("stage", f.stage.is_some()),
                ("status", f.status.is_some()),
                ("owner", f.owner.is_some()),
                ("title_contains", f.title_contains.is_some()),
            ];
            for field in declared
                .iter()
                .filter(|(_, present)| *present)
                .map(|(field, _)| field)
            {
                errs.push(refusal(
                    Some(name),
                    RefusalReason::FilterNotApplicable,
                    format!(
                        "this act does not narrow by `{field}`; narrowing by what a resource IS is \
                         the `find-resources-with` act — select with it and pipe the result in as a \
                         bound. Applying it here would answer a different question than the one \
                         asked"
                    ),
                ));
            }
        }
    }
    if !inv.properties.is_empty() {
        errs.push(refusal(
            Some(name),
            RefusalReason::FilterNotApplicable,
            "this door does not yet apply property predicates — the compiler emits no slot for \
             them, and a predicate that narrows nothing is a silent substitution",
        ));
    }
    // **The unconditional `edge_filter` refusal is RETIRED** `[2026-08-14]`. It read: "this door
    // does not yet apply edge filters — the only act that admits one still compiles to the absent
    // placeholder". Both halves have stopped being true: `follow-from` compiles to
    // `__temper_ungated_follow_from`, whose `p_edge_kinds`/`p_labels` are `EdgeFilter`'s two axes.
    //
    // What survives is the PER-ACT check further down — `accepts_filters.contains(&FilterField::
    // Edge)` — which refuses an edge filter on every act that does not traverse an edge. That check
    // was dead while this one refused everything first, so retiring this one is what puts it in
    // service; the test that pins it is what makes the retirement safe rather than merely smaller.
    //
    // `inv.properties` is deliberately NOT retired with it `[decided — 2026-08-14, Pete]`: spec §7
    // is OPEN — where a property
    // predicate's container lives is unsettled — and the compiler still emits no slot for one.

    // Bound terms. A ceiling is NOT a refusal — it clamps and is disclosed at execution. A term
    // outside the range the fragment can express IS a refusal, and must be one HERE. The negative
    // arm is malformed whatever the act, so it is the shape pass's; these two are this door's.
    //
    // `[added — 2026-08-09]` The membership check was the only one, and `applied_terms` clamps only
    // DOWNWARD against a ceiling — and `offset` has no ceiling on any act, so `3_000_000_000`
    // passed straight through to the statement and came back as "integer out of range": a caller's
    // error rendered as a server fault, one layer below where it was decidable. Found in review.
    for (term, value) in &inv.terms {
        if *value > i64::from(i32::MAX) {
            // The fragments take `int`, not `bigint`. A value above that range is not a large
            // page — it is a value the mechanic cannot express, and clamping it silently would
            // answer a different question than the one asked.
            errs.push(refusal(
                Some(name),
                RefusalReason::BoundTermNotApplicable,
                format!(
                    "the `{}` bound term is served by a 32-bit slot; {value} is outside the \
                     range this act can express",
                    term_wire_name(term)
                ),
            ));
        }
    }
    for term in inv.terms.keys() {
        if !decl.accepts_bound_terms.contains(term) {
            errs.push(refusal(
                Some(name),
                RefusalReason::BoundTermNotApplicable,
                format!(
                    "act does not admit the `{}` bound term",
                    term_wire_name(term)
                ),
            ));
        }
    }

    // Filters — declined, never silently ignored.
    if inv.resource_filter.is_some() && !decl.accepts_filters.contains(&FilterField::Resource) {
        errs.push(refusal(
            Some(name),
            RefusalReason::FilterNotApplicable,
            "act does not admit a resource filter",
        ));
    }
    // **The surviving edge-filter refusal, and the only one now** `[2026-08-14]`. It was
    // unreachable while an unconditional site above refused every edge filter first; retiring that
    // site is what put this one into service.
    //
    // Its message was "act does not admit an edge filter" — true, and it named neither the
    // narrowing nor where the capability lives, which the contract's own rule ("declined, never
    // ignored", naming what was declined) asks for and every neighbouring refusal does.
    if inv.edge_filter.is_some() && !decl.accepts_filters.contains(&FilterField::Edge) {
        errs.push(refusal(
            Some(name),
            RefusalReason::FilterNotApplicable,
            format!(
                "act `{}` does not traverse edges, so it cannot apply edge filters; narrowing \
                 which edges a walk follows belongs to `follow-from`, whose `edge_filter` \
                 constrains the hops themselves. Applying it here would answer a different \
                 question than the one asked",
                act_wire_name(&inv.act)
            ),
        ));
    }
}
