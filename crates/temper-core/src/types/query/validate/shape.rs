//! Expressibility — is this a well-formed composition?
//!
//! **This module may not consult act declarations.** A refusal raised here must be true of the
//! plan and the published wire contract alone, so that a client running it against a NEWER server
//! cannot refuse a plan that server would run. Version skew is structural in this project: a
//! released CLI binary carries a `search_family()` older than the server's, and
//! `CALLABLE_FRAGMENTS` — which decides `NotSeparablyReachable` — is exactly what later beats
//! widen.
//!
//! **That rule wants two guards, because one is not enough.** Scanning this module's source for a
//! route to the act registry is the obvious one, and it is not sufficient: six sites in the
//! capability pass read no declaration at all and are nonetheless pure door capability — each
//! refuses something THIS door has not built (a compiler slot, a fragment parameter, a hydrated
//! section), and [`super::capability`] lists the six and what retires each — so a source scan
//! alone would happily let them sit here. What this pass raises has to be pinned as
//! well, and **pinned per SITE — counts, not a set of reasons.**
//!
//! **Two variants straddle the seam**, and each is why the pin is over sites rather than variants:
//!
//! - [`RefusalReason::BoundTermNotApplicable`] — its negative-value site is here; its range and
//!   admission sites are in [`super::capability`].
//! - [`RefusalReason::AnchorTakesOneId`] — its ZERO-id site is here; its more-than-one site is in
//!   [`super::capability`], because a pair holding one id is today's fragment shape and an
//!   `anchor_ids uuid[]` retires it.
//!
//! Over a SET, either module's site migrating into this one would change nothing, because the
//! reason is in the set already. Over counts, this module's tally for that reason goes 1 → 2 and
//! the guard fails. The classification is a judgment, and a judgment needs a pin rather than an
//! inference.
//!
//! [`RefusalReason::FilterNotApplicable`] is not a second straddling variant, despite looking like
//! one — all six of its sites are capability. What differs among those six is whether the
//! limitation is permanent or not-yet-built, which is a fact about *retirement*, not about which
//! pass may raise it.
//!
//! The converse rule, and what lives on the other side of the seam, is in [`super::capability`].

use std::collections::{BTreeMap, BTreeSet};

use crate::types::query::act::ActName;
use crate::types::query::composition::{Composition, StageNode};
use crate::types::query::disposition::RefusalReason;
use crate::types::query::envelope::ActInvocation;
use crate::types::query::filter::{PropertyOp, PropertySubject};
use crate::types::query::id_set::IdKind;
use crate::types::query::stage::{StageInput, StageName};

use super::{index_by_name, refusal, term_wire_name, PlanRefusal};

/// Kahn's topological sort over the resolvable edges. `None` iff a cycle prevents a total order.
fn topo_order(by_name: &BTreeMap<&str, &StageNode>) -> Option<Vec<StageNode>> {
    let mut indegree: BTreeMap<&str, usize> = by_name.keys().map(|k| (*k, 0usize)).collect();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (&name, node) in by_name {
        for up in node.upstream_names() {
            if by_name.contains_key(up.as_str()) {
                *indegree.get_mut(name).expect("name is in the map") += 1;
                dependents.entry(up.as_str()).or_default().push(name);
            }
        }
    }

    let mut queue: Vec<&str> = by_name
        .keys()
        .copied()
        .filter(|n| indegree[n] == 0)
        .collect();
    let mut ordered_names: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < queue.len() {
        let n = queue[i];
        i += 1;
        ordered_names.push(n);
        if let Some(deps) = dependents.get(n) {
            for &d in deps {
                let e = indegree.get_mut(d).expect("dependent is in the map");
                *e -= 1;
                if *e == 0 {
                    queue.push(d);
                }
            }
        }
    }

    if ordered_names.len() != by_name.len() {
        return None;
    }
    Some(
        ordered_names
            .into_iter()
            .map(|n| (*by_name[n]).clone())
            .collect(),
    )
}

/// Every expressibility refusal, plus the topological order when the DAG admits one.
///
/// The order is returned rather than recomputed by the caller because [`super::validate`] needs
/// exactly the same one, and two sorts of the same graph are two things that can disagree.
pub(super) fn validate_shape_indexed(
    c: &Composition,
) -> (Vec<PlanRefusal>, Option<Vec<StageNode>>) {
    let mut errs: Vec<PlanRefusal> = Vec::new();

    // **A composition with no stages asks nothing.** `[added — 2026-08-09]` The contract declares
    // `stages: minItems 1` and nothing enforced it: an empty plan validated cleanly, compiled to a
    // zero-arm statement saved only by a fallback branch, and the compiler's comment beside that
    // fallback claimed it was "unreachable through `validate`, which refuses an empty `stages`" —
    // which was simply untrue. Found in review. Refusing here is what makes that sentence true.
    if c.stages.is_empty() {
        errs.push(refusal(
            None,
            RefusalReason::NoStages,
            "a composition must declare at least one stage; this one asks nothing",
        ));
    }

    // **A composition with no returns answers nothing.** `[added — 2026-08-10]` The contract
    // declares `returns: minItems 1` and nothing enforced it: an empty `returns` compiled and ran,
    // answering 200 where the contract says 400 (audit finding F6). Composition-level, like
    // `no-stages` — the omission belongs to no stage.
    if c.outcome.returns.is_empty() {
        errs.push(refusal(
            None,
            RefusalReason::NoReturns,
            "a composition must return at least one stage; this one answers nothing",
        ));
    }

    // Distinct declared names (first wins) + duplicate detection. The map is built by
    // [`index_by_name`] so that this pass and the capability pass cannot index a duplicate-named
    // plan differently; the set below is its key set, tracked as the loop runs so the SECOND
    // occurrence of a name is the one refused.
    let by_name = index_by_name(c);
    let mut declared: BTreeSet<&str> = BTreeSet::new();
    for node in &c.stages {
        let n = node.name().as_str();
        if !declared.insert(n) {
            errs.push(refusal(
                Some(node.name()),
                RefusalReason::DuplicateStageName,
                format!("two stages share the name `{n}`"),
            ));
        }
    }

    // Combinator arity + dangling references.
    for node in &c.stages {
        if let StageNode::Combine(cn) = node {
            if cn.inputs.len() < 2 {
                errs.push(refusal(
                    Some(node.name()),
                    RefusalReason::CombinatorArity,
                    "a set combination needs two or more inputs",
                ));
            }
        }
        for up in node.upstream_names() {
            if !declared.contains(up.as_str()) {
                errs.push(refusal(
                    Some(node.name()),
                    RefusalReason::DanglingReference,
                    format!(
                        "stage `{}` references undeclared stage `{}`",
                        node.name().as_str(),
                        up.as_str()
                    ),
                ));
            }
        }
    }

    // A `returns` entry must name a declared stage.
    // **A stage may be named in `returns` at most once.**
    //
    // `[added — 2026-08-09]` A duplicate emitted one hit arm PER ENTRY, so every row of that stage
    // came back twice while its tally still said `produced: 1` — and because `returned` is keyed by
    // stage, only the LAST entry's `with` survived, silently discarding the other's. Two answers to
    // one question, one of them thrown away without a word. Found in review.
    let mut returned_once: BTreeSet<&str> = BTreeSet::new();
    for ret in &c.outcome.returns {
        if !returned_once.insert(ret.stage.as_str()) {
            errs.push(refusal(
                Some(&ret.stage),
                RefusalReason::DuplicateReturnStage,
                format!(
                    "stage `{}` is named more than once in `returns`; a stage answers once, and \
                     two entries would duplicate its rows while only one `with` survived",
                    ret.stage.as_str()
                ),
            ));
        }
    }

    for ret in &c.outcome.returns {
        // **A combinator may not be RETURNED.** `[added — 2026-08-09]` Its rows come from two or
        // more acts, so the stage has no single act, no single `orders_by`, and no single
        // `score_kind` — and a returned stage's rows carry exactly one of each. Hydrating a union
        // would put two acts' rows into ONE ordered list, which is the merged list
        // `no-cross-act-ranking` exists to make unrepresentable.
        //
        // It was reachable and silent: `ValidationOutcome::of` SKIPS a combinator when computing
        // `will_return` (so the promise simply omitted it), the compiler emitted a hit arm for it
        // anyway, and the assembler dropped every row for want of a score kind — answering
        // `disposition: answered` with an empty list and a tally saying rows existed. Found in
        // review. Combining stays legal; asking for the combined rows back does not.
        if matches!(by_name.get(ret.stage.as_str()), Some(StageNode::Combine(_))) {
            errs.push(refusal(
                Some(&ret.stage),
                RefusalReason::CombinatorNotReturnable,
                format!(
                    "stage `{}` combines other stages, so its rows have no single act to score \
                     them; return the stages it combines instead",
                    ret.stage.as_str()
                ),
            ));
        }
        if !declared.contains(ret.stage.as_str()) {
            errs.push(refusal(
                Some(&ret.stage),
                RefusalReason::UnknownReturnStage,
                format!("returns names undeclared stage `{}`", ret.stage.as_str()),
            ));
        }
    }

    // Per-stage checks run only when the graph is a graph — the incumbent shape of this function
    // and not a new rule. The composition- and returns-level findings above still stand beside a
    // cycle; what a cyclic plan does not additionally get is a finding per stage, because a plan
    // whose stages cannot be ordered is a plan that cannot be read.
    match topo_order(&by_name) {
        None => {
            errs.push(refusal(
                None,
                RefusalReason::Cycle,
                "the composition contains a cycle; a query DAG must be acyclic",
            ));
            (errs, None)
        }
        Some(ordered) => {
            for node in &c.stages {
                if let StageNode::Act(inv) = node {
                    check_act(inv, node.name(), &mut errs);
                }
            }
            (errs, Some(ordered))
        }
    }
}

/// The expressibility checks for one act node. Every axis is independent — a node can fail more
/// than one — so nothing short-circuits; a caller sees the whole picture.
/// `[2026-08-12]` No longer takes the `&Composition`. It needed one for exactly one reason —
/// reading the envelope's `intention` — and spec ⟨7⟩ moved that onto the node. **A per-node check
/// that cannot see the composition is the stronger shape**: it cannot accidentally let a sibling
/// stage's field satisfy this one's requirement, which is the defect the move exists to prevent.
fn check_act(inv: &ActInvocation, name: &StageName, errs: &mut Vec<PlanRefusal>) {
    // `ActName` is open (`act.rs:45-46`), so an unrecognized act name deserializes into
    // `Other` rather than failing serde. That makes this caller-reachable — a KNOWN variant with
    // no declaration would be an internal inconsistency, not a caller error, and the capability
    // pass owns that case.
    //
    // Shape rather than capability, and NOT because the check reads no declaration — that test is
    // the one this file's header names as insufficient. It is shape because catching a MISSPELLED
    // act name is worth more offline than the case below is worth avoiding: a plan naming
    // `find-abuot-within` is wrong against every server that will ever exist, and telling the
    // caller so without a round trip is the whole value of `temper query --check`.
    //
    // **DECLARED REMAINDER — a shape refusal that can fire wrongly against a newer server, kept
    // deliberately. It is the COSTLIER of the two; the anchor's zero arm below is the other, and
    // said to be one rather than left implied.** `[widened from "the one" — 2026-08-12, re-review]`
    // The two are not equivalent and the difference is the whole reason both are tolerable: this
    // one refuses a plan the newer server would have ANSWERED, while the zero anchor refuses one
    // that would have returned nothing. `ActName` is open in the direction that GROWS: when an
    // eighth act joins `search_family()`, a released CLI whose binary predates it deserializes that
    // name into `Other` and refuses `unknown_act` for a plan the current server would run. Nothing
    // here can tell that case apart from a typo, because the two are textually identical — which is
    // why the detail names both readings rather than asserting the wrong one. The alternative,
    // moving it to capability, buys the rare stale-binary case at the cost of the common typo, and
    // was declined; see the design's ⟨3⟩.
    if let ActName::Other(raw) = &inv.act {
        errs.push(refusal(
            Some(name),
            RefusalReason::UnknownAct,
            format!(
                "`{raw}` is not an act this binary knows — check the spelling, or update if your \
                 server is newer than it"
            ),
        ));
        return;
    }

    // What a CALLER-supplied set must be, whatever the act does with it. The upstream case has no
    // expressibility question — the kind an upstream stage produces is a declared fact, so both
    // checks below would need the registry to ask it, and neither applies: provenance is the
    // caller's responsibility (an upstream `survey` supplies it itself), and the anchor kinds are
    // accepted but never produced, so an upstream set is never anchor-kind.
    if let Some(StageInput::Caller { ids, .. }) = &inv.input {
        let kind = &ids.kind;
        if *kind == IdKind::Region && ids.provenance.is_none() {
            errs.push(refusal(
                Some(name),
                RefusalReason::MissingProvenance,
                "a region set must declare whether it is cogmap- or context-anchored",
            ));
        }

        // The anchor slot's CARDINALITY, checked separately from its kind because they are
        // different complaints: the kind is accepted and the count is not.
        //
        // **Only the ZERO arm is shape, and the reason is the DIRECTION OF FAILURE — not
        // impossibility.** `[split — 2026-08-12, Pete; reason corrected — 2026-08-12, re-review]`
        // One site used to refuse `len() != 1`, conflating two claims that fail opposite ways.
        //
        // Admitting a zero anchor TODAY would drop the scope and answer a WIDER question than the
        // caller asked — a silent widening, which is a correctness problem rather than a capability
        // one, and the class this validator exists to make impossible. Refusing it costs a stale
        // client nothing: the plan it declines would have returned nothing anyway. The many case is
        // the reverse — refusing it is what costs, the moment a fragment can take the set.
        //
        // **The impossibility argument was tried first and is FALSE**, recorded so it is not
        // re-derived. It ran: an anchor has no `'{}'`/`NULL` pair, so no fragment change makes an
        // empty anchor mean something. But `disposition.rs:74-77` says that about TODAY's
        // `(anchor_table, anchor_id)`, not about all fragment futures — and the very widening cited
        // to retire the many arm falsifies it, since an `anchor_ids uuid[]` would give an empty
        // anchor exactly the `'{}'` = bounded-to-nothing meaning `IdKind::Resource` already carries
        // (`query_plan.rs` binds a caller resource array unrefused at any length, zero included).
        // Under that widening BOTH arms retire. So this arm, like `UnknownAct` above, can in
        // principle fire against a widened server; what keeps it here is that its wrong-firing
        // costs an empty answer, where the many arm's costs a real one.
        //
        // Supplying SEVERAL is refused only because today's fragments take the pair, which is a
        // parameter shape — so that arm is capability, and lives in [`super::capability`] beside
        // the structurally identical `f.doc_type.len() > 1`.
        if matches!(kind, IdKind::Cogmap | IdKind::Context) && ids.ids.is_empty() {
            errs.push(refusal(
                Some(name),
                RefusalReason::AnchorTakesOneId,
                format!(
                    "a `{kind:?}` bound must name the one thing it scopes to; this stage \
                     named none. Accepting it would silently widen your question to the \
                     unscoped one"
                ),
            ));
        }
    }

    // A negative bound term is malformed whatever the act admits — a count below zero is not a
    // narrowing this door has yet to build.
    //
    // `[corrected — 2026-08-12]` This said "counts rows", and so did the refusal it raises. That is
    // true of two of the three terms and false of the third: `scalars.rs:47` declares
    // `BoundTerm::Regions` a FUNNEL WIDTH, `survey`'s only term and one with no rows to count —
    // which is precisely what `survey_declines_limit_because_its_bound_is_a_funnel_width` is built
    // on. What is true of all three is that each is a non-negative count of something
    // (`scalars.rs:30-31`: rows, rows skipped, regions), so that is what the detail now says.
    //
    // `[added — 2026-08-09]` The membership check was the only one, and `applied_terms` clamps only
    // DOWNWARD against a ceiling, so a negative value passed straight through to the statement:
    // `limit: -1` reached Postgres as `LIMIT must not be negative` and surfaced as a 500. Same
    // class as the empty-intention refusal below — a caller's error rendered as a server fault, one
    // layer below where it was decidable. Found in review.
    for (term, value) in &inv.terms {
        if *value < 0 {
            errs.push(refusal(
                Some(name),
                RefusalReason::BoundTermNotApplicable,
                format!(
                    "the `{}` bound term is a count and cannot be negative; this stage \
                     supplied {value}",
                    term_wire_name(term)
                ),
            ));
        }
    }

    // Every find act refuses without a threaded intention. For `find-about-*` the reason is that
    // the server does not supply the QUESTION on the caller's behalf — "no question was asked" and
    // "the question could not be embedded" stay distinct. `find-exact` needs the intention for a
    // different reason: its query TEXT is `query_find_exact`'s `p_query`, and there is nowhere else
    // to get it.
    //
    // `[corrected — 2026-08-12]` This said the server "does not embed on the caller's behalf",
    // which is false in the other direction: the server DOES embed, which is why
    // `RefusalReason::EmbeddingUnavailable` exists for its failed attempt
    // (`disposition.rs:94-96`: *"It does not fire for a missing embedding — the server computes
    // one, because API callers structurally cannot"*) and why the `[widened — 2026-08-09]` note
    // further down this same block describes `compile` reading a `None` as "the server tried and
    // failed". What it will not supply is the question itself.
    //
    // `[widened at beat D — 2026-08-08]` `FindExact` was missing here, which was invisible while
    // the find acts were `NotSeparablyReachable` and became a real defect the moment the compiler
    // could emit them: `validate` returned Ok and `compile` returned Err(MissingIntention) for the
    // same plan. That is precisely the validator/emitter disagreement `CALLABLE_FRAGMENTS` was
    // reshaped into a shared map to make impossible, reappearing on a different axis — the map
    // makes the two agree about WHICH ACTS are reachable, and nothing was making them agree about
    // WHAT EACH ACT REQUIRES.
    //
    // Shape rather than capability, and NOT because the check happens to read no declaration —
    // that test is the one the header at the top of this file names as insufficient. The argument
    // is about published meaning: a find act's need for a question is part of what the act IS on
    // the contract, not something this server has yet to build. `find-exact`'s query text becomes
    // `p_query` and has no other source, and the `find-about-*` acts refuse rather than let the
    // server invent the question. No server builds its way out of either.
    if matches!(
        inv.act,
        ActName::FindExact | ActName::FindAboutAnywhere | ActName::FindAboutWithin
    ) {
        // `[widened — 2026-08-09]` An empty or whitespace-only query is the SAME omission as an
        // absent intention, and leaving it out here sent it somewhere false: the server-side embed
        // declines to embed nothing, `compile` reads the resulting `None` as "the server tried and
        // failed", and the caller was told `embedding_unavailable` — a server fault, for a question
        // they never asked. The server never attempted anything. Found in review.
        //
        // `[2026-08-13]` The function that arm named, `resolve_embedding`, is gone; its successor
        // is `temper-services`' `text_to_embed`, which holds the same arm. Deliberately BOTH: this
        // pass is what makes it structural — under `prepare` an empty question never reaches an
        // embed at all — and that one is the contract of a function this crate cannot see.
        //
        // `[2026-08-12]` Reads THIS STAGE's intention, not the composition's — spec ⟨7⟩ moved the
        // field onto `ActInvocation`. The check was already per-invocation and already attached its
        // refusal to `name`, so the move is one field access; what changes is that a sibling stage's
        // intention no longer satisfies this one. That is the point: two find stages may now ask
        // different questions, and each must bring its own.
        let missing = match inv.intention.as_ref() {
            None => Some("this find act carries no intention, and a find act needs a question"),
            Some(i) if i.query.trim().is_empty() => {
                Some("this find act's intention carries no question; its query text is empty")
            }
            Some(_) => None,
        };
        if let Some(detail) = missing {
            errs.push(refusal(Some(name), RefusalReason::MissingIntention, detail));
        }
    }

    // Property predicates: open subject vocabulary, non-empty key, non-empty `contains`. All three
    // are about the predicate as written — whether this door APPLIES predicates at all is the
    // capability pass's question.
    for p in &inv.properties {
        if let PropertySubject::Other(s) = &p.subject {
            errs.push(refusal(
                Some(name),
                RefusalReason::UnknownFilterValue,
                format!("`{s}` is not a queryable property subject"),
            ));
        }
        if p.key.is_empty() {
            errs.push(refusal(
                Some(name),
                RefusalReason::EmptyPropertyKey,
                "a property predicate needs a key",
            ));
        }
        if let PropertyOp::Contains { values } = &p.op {
            if values.is_empty() {
                errs.push(refusal(
                    Some(name),
                    RefusalReason::EmptyContains,
                    "`contains` with no values narrows nothing",
                ));
            }
        }
    }
}
