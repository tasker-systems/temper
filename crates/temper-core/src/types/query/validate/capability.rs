//! Capability — the shape is fine, and this server has not built it yet.
//!
//! Everything here may move as beats land, which is why none of it may be raised by a client
//! against a server it does not share a binary with. Note that reading a declaration is NOT what
//! makes a check belong here: the four "this door does not apply" `FilterNotApplicable` sites, the
//! [`SectionNotAvailable`](RefusalReason::SectionNotAvailable) loop, and the more-than-one
//! [`AnchorTakesOneId`](RefusalReason::AnchorTakesOneId) arm read no declaration at all and are
//! pure door capability. Only two of them say *"does not yet apply"* outright; the rest read like
//! permanent structural facts and are not. Six sites, and no two are retired by the same thing:
//!
//! - property predicates — Task 10b;
//! - edge filters — Task 11;
//! - the six-field resource narrowing — a compiler slot that does not exist yet;
//! - the one-value doc-type narrowing — a fragment parameter a later fragment can widen;
//! - the one-id anchor slot — the same class as the line above and a DIFFERENT parameter: the
//!   fragments' `(anchor_table, anchor_id)` pair, retired by an `anchor_ids uuid[]`;
//! - [`SectionNotAvailable`](RefusalReason::SectionNotAvailable) — a widened
//!   [`ReturnSpec::ADMITTED_SECTIONS`].
//!
//! **The pass has two halves, and only one of them is gated on the topology.** [`validate_stages`]
//! reads the stage graph; [`validate_returns`] compares `with` against a constant and reads no
//! graph at all, so [`super::validate`] runs it whatever the plan's shape.
//!
//! The converse rule, and what lives on the other side of the seam, is in [`super::shape`].

use std::collections::BTreeMap;

use crate::types::query::act::BuildState;
use crate::types::query::composition::{Composition, ReturnSpec, StageNode};
use crate::types::query::disposition::RefusalReason;
use crate::types::query::envelope::ActInvocation;
use crate::types::query::filter::FilterField;
use crate::types::query::id_set::IdKind;
use crate::types::query::registry::declaration;
use crate::types::query::stage::{StageInput, StageName, StageRelation};
use crate::types::resource_view::ResourceSection;

use super::{act_wire_name, emitted_fragment_for, refusal, term_wire_name, PlanRefusal};

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
    if let Some(input) = &inv.input {
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
    // parameter's shape, exactly like `f.doc_type.len() > 1` below, and exactly what an
    // `anchor_ids uuid[]` would retire. Held in the shape pass it would let a client refuse a plan
    // a widened server runs.
    //
    // Read off the CALLER's set only, which is the same restriction the shape half carries: what an
    // upstream stage produces is a declared fact, and the anchor kinds are accepted but never
    // produced, so an upstream set is never anchor-kind.
    if let Some(StageInput::Caller { ids, .. }) = &inv.input {
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
    if let Some(f) = &inv.resource_filter {
        let declared: &[(&str, bool)] = &[
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
                    "this door does not apply the `{field}` narrowing — the act admits it, the \
                     compiler has no slot for it, and ignoring it would answer a different question \
                     than the one asked"
                ),
            ));
        }
        // The fragment's `p_doc_type` is a single `text`, not an array, so a multi-value doc-type
        // filter is inexpressible rather than merely unimplemented. Same shape as the anchor slot:
        // narrowing to the first of them would answer a different question and look like a
        // successful narrowing.
        if f.doc_type.len() > 1 {
            errs.push(refusal(
                Some(name),
                RefusalReason::FilterNotApplicable,
                format!(
                    "this door's doc-type narrowing holds exactly one value; this stage supplied {}",
                    f.doc_type.len()
                ),
            ));
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
    if inv.edge_filter.is_some() {
        errs.push(refusal(
            Some(name),
            RefusalReason::FilterNotApplicable,
            "this door does not yet apply edge filters — the only act that admits one still \
             compiles to the absent placeholder",
        ));
    }

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
    if inv.edge_filter.is_some() && !decl.accepts_filters.contains(&FilterField::Edge) {
        errs.push(refusal(
            Some(name),
            RefusalReason::FilterNotApplicable,
            "act does not admit an edge filter",
        ));
    }
}
