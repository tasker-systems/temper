//! The pure static validator.
//!
//! It decides everything a composition can be refused for BEFORE any SQL exists, against
//! `search_family()` and no database: topology (cycles, dangling references, duplicate names,
//! combinator arity, unknown return stages) and declaration conformance (input kinds, bound terms,
//! filters, region provenance, the threaded intention, property predicates, and surface
//! reachability). It extends contract §4.1.1's static-refusal rule from bound terms to the whole
//! plan shape (spec §5).
//!
//! Two properties are the point. `validate` returns **every** refusal, not the first — a caller
//! repairing a plan should see all of it in one round trip. And [`ValidatedComposition`] is
//! parse-don't-validate: its fields are private and this is the only constructor, so the compiler
//! cannot be handed a plan that skipped these checks.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use super::act::{ActName, ActQuantity, BuildState, Disclosure};
use super::composition::{Composition, ReturnSpec, StageNode};
use super::disposition::RefusalReason;
use super::envelope::ActInvocation;
use super::filter::{FilterField, PropertyOp, PropertySubject};
use super::hits::ScoreKind;
use super::id_set::IdKind;
use super::registry::declaration;
use super::stage::{ProducedVariant, StageInput, StageName, StageRelation};
use crate::types::resource_view::ResourceSection;

/// Declared mechanic (`served_by`) → the SQL function the compiler actually emits.
///
/// An act whose `served_by` is absent here is declared and built but not reachable from THIS
/// surface — refused as [`RefusalReason::NotSeparablyReachable`], never as `NotImplemented`.
///
/// A MAP rather than a set since beat D, and the two names differ for the find acts on purpose:
/// `served_by` must keep naming what `/api/search` calls, because that is the mechanic the
/// declaration describes, while `/api/query` emits the fragment that accepts `p_bound_ids uuid[]`.
/// They remain ONE BODY — `search_exact` delegates to `query_find_exact`, which delegates to
/// `__temper_ungated_find_exact` — but they are three signatures, and the declaration describes the
/// door rather than the compiler.
///
/// The emitted names are the UNGATED cores (`20260808000030`), not the gated twins. `/api/query`
/// hoists the visibility relation into one CTE and hands it to every stage, which is only possible
/// against a fragment that does not gate internally; the twins remain what `/api/search` reaches.
/// A caller of this map must therefore already hold an RBAC verdict — in this crate the map only
/// decides reachability, and the sole emitter of these names is
/// `temper_substrate::readback::query_plan::emit_ungated_core_call`, which supplies that verdict
/// itself rather than taking it as an argument.
///
/// Membership is what decides `NotSeparablyReachable`. It is keyed on served-by names and NEVER on
/// `build_state`: the two `Fused` declarations (`follow-from`, `survey`) are among the reachable
/// ones, so a rule keyed on that discriminant would refuse the wrong acts.
///
/// `follow-from` and `survey` map to the deliberately-absent placeholder. That is honest rather
/// than sloppy: their mechanics exist and are reachable *in principle*, but their fragments take
/// arguments no slot supplies (`p_depth`/`p_gamma`, `p_lens`), so the builder emits a function that
/// does not exist and Postgres errors loudly instead of a guessed value returning plausible rows.
/// Keeping them here — rather than dropping them, which would make them refuse statically —
/// preserves the beat-C behaviour their tests pin.
const CALLABLE_FRAGMENTS: &[(&str, &str)] = &[
    ("search_exact", "__temper_ungated_find_exact"),
    ("search_wide", "__temper_ungated_find_wide"),
    ("search_graph_expand", "__temper_unbound_act"),
    ("wayfind_region_scores", "__temper_unbound_act"),
];

/// The fragment the compiler emits for a declared mechanic, or `None` if this surface cannot reach
/// it. The compiler reads this so the reachability rule and the emission cannot disagree — two
/// lists would drift, and the drift would be a stage that validates and then compiles to nothing.
pub fn emitted_fragment_for(served_by: &str) -> Option<&'static str> {
    CALLABLE_FRAGMENTS
        .iter()
        .find(|(mechanic, _)| *mechanic == served_by)
        .map(|(_, fragment)| *fragment)
}

/// One reason a plan is not executable. Static — no database was consulted.
///
/// **On the wire**, in `ErrorBody.error.details.refusals`: a rejected composition answers 400 with
/// ALL of these at once, never just the first, because repairing a plan one refusal per round trip
/// is the experience that design avoids. It carries the generation derives for that reason — it
/// went without them while nothing could return it, and a door is now being built that does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct PlanRefusal {
    /// The stage it attaches to, when it attaches to one.
    ///
    /// `None` for a refusal about the composition as a whole — a cycle, a dangling reference, a
    /// duplicate name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<StageName>,
    pub reason: RefusalReason,
    /// Human-readable, at the depth the asker's standing allows.
    pub detail: String,
}

/// A composition that has passed every static check, in topological order.
///
/// Parse-don't-validate: the fields are private and [`validate`] is the only constructor, so a
/// compiler that accepts this type cannot be handed an unvalidated plan.
#[derive(Debug, Clone)]
pub struct ValidatedComposition {
    composition: Composition,
    ordered: Vec<StageNode>,
}

impl ValidatedComposition {
    /// Nodes in dependency order — every node appears after all of its upstreams.
    pub fn ordered(&self) -> &[StageNode] {
        &self.ordered
    }

    pub fn composition(&self) -> &Composition {
        &self.composition
    }

    pub fn returns(&self) -> &[ReturnSpec] {
        &self.composition.outcome.returns
    }
}

fn refusal(
    stage: Option<&StageName>,
    reason: RefusalReason,
    detail: impl Into<String>,
) -> PlanRefusal {
    PlanRefusal {
        stage: stage.cloned(),
        reason,
        detail: detail.into(),
    }
}

/// An act's name as the caller wrote it, for a refusal to quote back.
///
/// Through serde rather than a second match, so a refusal can never name an act by a spelling the
/// wire does not accept — which would tell a caller to retry something unparseable.
fn act_wire_name(act: &ActName) -> String {
    serde_json::to_string(act)
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|_| format!("{act:?}"))
}

/// The kind an upstream node produces, walking a combinator to its first input. `None` for a
/// dangling reference (already refused as topology) or an act that produces nothing.
fn produced_kind_of(name: &str, by_name: &BTreeMap<&str, &StageNode>) -> Option<IdKind> {
    match by_name.get(name)? {
        StageNode::Act(inv) => declaration(&inv.act)?.produces,
        StageNode::Combine(c) => produced_kind_of(c.inputs.first()?.as_str(), by_name),
    }
}

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

/// Declaration-driven checks for one act node. Every axis is independent — a node can fail more than
/// one — so nothing short-circuits; a caller sees the whole picture.
fn check_act(
    inv: &ActInvocation,
    name: &StageName,
    c: &Composition,
    by_name: &BTreeMap<&str, &StageNode>,
    errs: &mut Vec<PlanRefusal>,
) {
    let Some(decl) = declaration(&inv.act) else {
        errs.push(refusal(
            Some(name),
            RefusalReason::Other("unknown-act".to_string()),
            format!("`{:?}` is not a known act", inv.act),
        ));
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
        let (incoming, from_caller, provenance) = match input {
            StageInput::Caller { ids, .. } => (Some(ids.kind.clone()), true, ids.provenance),
            StageInput::Upstream { stage, .. } => {
                (produced_kind_of(stage.as_str(), by_name), false, None)
            }
        };
        if let Some(kind) = incoming {
            // Provenance is the CALLER's responsibility; an upstream `survey` supplies it itself.
            if from_caller && kind == IdKind::Region && provenance.is_none() {
                errs.push(refusal(
                    Some(name),
                    RefusalReason::MissingProvenance,
                    "a region set must declare whether it is cogmap- or context-anchored",
                ));
            }
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

            // The anchor slot's CARDINALITY, checked separately from its kind because they are
            // different complaints: the kind is accepted and the count is not. Only a CALLER can
            // supply one of these — `cogmap` and `context` are accepted but never produced, so an
            // upstream set is never anchor-kind (see the chainability relation in the contract).
            if let StageInput::Caller { ids, .. } = input {
                if matches!(ids.kind, IdKind::Cogmap | IdKind::Context) && ids.ids.len() != 1 {
                    errs.push(refusal(
                        Some(name),
                        RefusalReason::AnchorTakesOneId,
                        format!(
                            "a `{kind:?}` bound is served by the anchor pair, which holds exactly \
                             one id; this stage supplied {}. Anchoring on one of them would answer \
                             a different question than the one asked",
                            ids.ids.len()
                        ),
                    ));
                }
            }
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

    // Bound terms. A ceiling is NOT a refusal — it clamps and is disclosed at execution.
    for term in inv.terms.keys() {
        if !decl.accepts_bound_terms.contains(term) {
            errs.push(refusal(
                Some(name),
                RefusalReason::BoundTermNotApplicable,
                format!("act does not admit the `{term:?}` bound term"),
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

    // Every find act refuses without a threaded intention. For `find-about-*` the reason is that
    // the server does not embed on the caller's behalf — "I chose not to embed" and "I cannot
    // embed" stay distinct. `find-exact` needs the intention for a different reason: its query TEXT
    // is `query_find_exact`'s `p_query`, and there is nowhere else to get it.
    //
    // `[widened at beat D — 2026-08-08]` `FindExact` was missing here, which was invisible while
    // the find acts were `NotSeparablyReachable` and became a real defect the moment the compiler
    // could emit them: `validate` returned Ok and `compile` returned Err(MissingIntention) for the
    // same plan. That is precisely the validator/emitter disagreement `CALLABLE_FRAGMENTS` was
    // reshaped into a shared map to make impossible, reappearing on a different axis — the map
    // makes the two agree about WHICH ACTS are reachable, and nothing was making them agree about
    // WHAT EACH ACT REQUIRES.
    if matches!(
        inv.act,
        ActName::FindExact | ActName::FindAboutAnywhere | ActName::FindAboutWithin
    ) {
        // `[widened — 2026-08-09]` An empty or whitespace-only query is the SAME omission as an
        // absent intention, and leaving it out here sent it somewhere false: `resolve_embedding`
        // declines to embed nothing, `compile` reads the resulting `None` as "the server tried and
        // failed", and the caller was told `embedding_unavailable` — a server fault, for a question
        // they never asked. The server never attempted anything. Found in review.
        let missing = match c.intention.as_ref() {
            None => Some("a find act requires a threaded intention"),
            Some(i) if i.query.trim().is_empty() => Some(
                "a find act requires a threaded intention with a question in it; this one is empty",
            ),
            Some(_) => None,
        };
        if let Some(detail) = missing {
            errs.push(refusal(Some(name), RefusalReason::MissingIntention, detail));
        }
    }

    // Property predicates: open subject vocabulary, non-empty key, non-empty `contains`.
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
                RefusalReason::Other("empty-property-key".to_string()),
                "a property predicate needs a key",
            ));
        }
        if let PropertyOp::Contains { values } = &p.op {
            if values.is_empty() {
                errs.push(refusal(
                    Some(name),
                    RefusalReason::Other("empty-contains".to_string()),
                    "`contains` with no values narrows nothing",
                ));
            }
        }
    }
}

/// What a composition WOULD return, derived from the act declarations without running anything.
///
/// This exists because `QueryResponse.returned` is an open map and has to be: its keys are whatever
/// the caller named their stages, and the shape under each depends on which act that stage runs.
/// That is a dependency from the request to the response, and **OpenAPI has no way to express it**.
/// A reader of the schema alone learns only *"some stage names, each holding one of four
/// variants."*
///
/// This closes it for one specific composition. Every fact it needs is already declared — an act's
/// `produces` and `orders_by` give the output variant, and its `discloses` gives which optional
/// fields will be filled — so it COMPUTES rather than guesses, and touches no rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ValidationOutcome {
    /// The stages in the order they will run — the topological sort, which is not the order they
    /// were listed in and which a DAG does not otherwise reveal.
    pub order: Vec<StageName>,
    /// Exactly the keys `QueryResponse.returned` will carry, and what will be under each.
    ///
    /// **This is the part the schema cannot say for itself.**
    pub will_return: BTreeMap<StageName, WillReturn>,
}

/// What one returned stage will carry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct WillReturn {
    pub act: ActName,
    /// Which `StageOutput` variant this stage will carry: resources, or regions.
    pub produced: ProducedVariant,
    /// The score kind every row of this stage will carry, typed exactly as the rows carry it.
    ///
    /// The envelope tags currency only, so `produced` alone does not distinguish two acts that
    /// both return resources — this is what completes the answer, and it is the SAME type a hit
    /// holds, so a caller compares it directly rather than across a translation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_kind: Option<ScoreKind>,
    /// The quantity its rows will be ordered by, with its range. Absent for an act that orders
    /// nothing — which, since every returnable act orders something, does not happen today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orders_by: Option<ActQuantity>,
    /// Which optional fields will be FILLED rather than null, for this act.
    ///
    /// Read it and you know in advance whether asking for `input_contributed` or `located_at` is
    /// worth doing — rather than discovering a null in a response and having to guess whether it
    /// means *cannot* or *none*.
    pub discloses: Vec<Disclosure>,
}

impl ValidationOutcome {
    /// Compute what this composition would return. No database, no execution.
    ///
    /// Takes a [`ValidatedComposition`] rather than a `Composition` on purpose: parse-don't-validate
    /// means the topological order is already computed and every declaration-driven check has
    /// already passed, so this cannot be asked about a plan that would be refused.
    pub fn of(v: &ValidatedComposition) -> ValidationOutcome {
        let order = v
            .ordered()
            .iter()
            .map(|n| n.name().clone())
            .collect::<Vec<_>>();

        let by_name: BTreeMap<&str, &StageNode> = v
            .composition()
            .stages
            .iter()
            .map(|n| (n.name().as_str(), n))
            .collect();

        let will_return = v
            .returns()
            .iter()
            .filter_map(|ret| {
                // A combinator node names no act, so it has no declared response shape. It cannot
                // be a returned stage today — `emit_combine_body` passes ids through and no act
                // hydrates them — so it is SKIPPED rather than guessed at. The absence is visible:
                // a caller sees a key they asked for missing from `will_return`, which is the
                // honest answer to "what will this give me", not a fabricated one.
                let StageNode::Act(inv) = by_name.get(ret.stage.as_str())? else {
                    return None;
                };
                let decl = declaration(&inv.act)?;
                Some((
                    ret.stage.clone(),
                    WillReturn {
                        act: inv.act.clone(),
                        produced: decl.produced_variant()?,
                        score_kind: decl.score_kind(),
                        orders_by: decl.orders_by.clone(),
                        discloses: decl.discloses.clone(),
                    },
                ))
            })
            .collect();

        ValidationOutcome { order, will_return }
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

/// Validate a composition against `search_family()` and the plan's own topology. Returns ALL
/// refusals, never just the first.
pub fn validate(c: &Composition) -> Result<ValidatedComposition, Vec<PlanRefusal>> {
    let mut errs: Vec<PlanRefusal> = Vec::new();

    // Distinct declared names (first wins) + duplicate detection.
    let mut by_name: BTreeMap<&str, &StageNode> = BTreeMap::new();
    let mut declared: BTreeSet<&str> = BTreeSet::new();
    for node in &c.stages {
        let n = node.name().as_str();
        if declared.insert(n) {
            by_name.insert(n, node);
        } else {
            errs.push(refusal(
                Some(node.name()),
                RefusalReason::Other("duplicate-stage-name".to_string()),
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
                    RefusalReason::Other("combinator-arity".to_string()),
                    "a set combination needs two or more inputs",
                ));
            }
        }
        for up in node.upstream_names() {
            if !declared.contains(up.as_str()) {
                errs.push(refusal(
                    Some(node.name()),
                    RefusalReason::Other("dangling-reference".to_string()),
                    format!(
                        "stage `{}` references undeclared stage `{}`",
                        node.name().as_str(),
                        up.as_str()
                    ),
                ));
            }
        }
    }

    // A `returns` entry must name a declared stage, and may only ask for sections this door
    // hydrates.
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
                RefusalReason::Other("combinator-not-returnable".to_string()),
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
                RefusalReason::Other("unknown-return-stage".to_string()),
                format!("returns names undeclared stage `{}`", ret.stage.as_str()),
            ));
        }
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

    match topo_order(&by_name) {
        None => errs.push(refusal(
            None,
            RefusalReason::Other("cycle".to_string()),
            "the composition contains a cycle; a query DAG must be acyclic",
        )),
        Some(ordered) => {
            for node in &c.stages {
                if let StageNode::Act(inv) = node {
                    check_act(inv, node.name(), c, &by_name, &mut errs);
                }
            }
            if errs.is_empty() {
                return Ok(ValidatedComposition {
                    composition: c.clone(),
                    ordered,
                });
            }
        }
    }

    Err(errs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::graph::EdgeKind;
    use crate::types::ids::CogmapId;
    use crate::types::query::composition::{
        CombineNode, CombineOp, Composition, Intention, OutcomeDeclaration,
    };
    use crate::types::query::envelope::ActInvocation;
    use crate::types::query::filter::{EdgeFilter, PropertyPredicate};
    use crate::types::query::id_set::{IdKind, IdProvenance, IdSet};
    use crate::types::query::scalars::BoundTerm;
    use std::collections::BTreeMap;

    /// The relation that makes a plan legal for this act: `follow-from` seeds, everything else
    /// bounds. Chosen so the input-kind check reads the right acceptance list — a test about
    /// topology should not fail on a relation mismatch it did not mean to introduce.
    fn natural_relation(a: ActName) -> StageRelation {
        if a == ActName::FollowFrom {
            StageRelation::Seed
        } else {
            StageRelation::Bound
        }
    }

    /// A minimal legal act node.
    ///
    /// The relation is REWRITTEN onto whatever input it is handed, rather than being a parameter
    /// of every call site. Callers build inputs with [`caller_ids`] / [`upstream`] and get the
    /// relation their act needs; a test that wants a DELIBERATELY wrong relation uses
    /// [`act_with_relation`] and says so in its name.
    fn act(name: &str, a: ActName, input: Option<StageInput>) -> StageNode {
        let relation = natural_relation(a.clone());
        act_with_relation(name, a, input, relation)
    }

    fn act_with_relation(
        name: &str,
        a: ActName,
        input: Option<StageInput>,
        relation: StageRelation,
    ) -> StageNode {
        let input = input.map(|i| match i {
            StageInput::Caller { ids, .. } => StageInput::Caller { relation, ids },
            StageInput::Upstream { stage, .. } => StageInput::Upstream { relation, stage },
        });
        StageNode::Act(ActInvocation {
            name: StageName::parse(name).unwrap(),
            act: a,
            input,
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    }

    fn plan(stages: Vec<StageNode>, returns: Vec<&str>) -> Composition {
        plan_returning(
            stages,
            returns
                .into_iter()
                .map(|s| ReturnSpec {
                    stage: StageName::parse(s).unwrap(),
                    with: vec![],
                })
                .collect(),
        )
    }

    fn plan_returning(stages: Vec<StageNode>, returns: Vec<ReturnSpec>) -> Composition {
        Composition {
            outcome: OutcomeDeclaration { returns },
            intention: None,
            meta_detail: Default::default(),
            bounds: BTreeMap::new(),
            stages,
        }
    }

    /// A caller-supplied bound of `kind`, carrying **one** id, with region provenance where the
    /// kind requires it so the set is well-formed. The relation is a placeholder — [`act`]
    /// overwrites it with the one the act actually accepts.
    ///
    /// `[was `ids: vec![]` — 2026-08-09]` The empty list made every anchor-kind fixture here an
    /// UNEXECUTABLE plan that happened to validate: the compiler's `let [id] = ids.as_slice()`
    /// matches exactly one element, so zero fell to its `else` and refused fatally. Nothing noticed,
    /// because no test in this file compiles anything. When the cardinality check moved here the
    /// fixtures started failing, which is the check working — these tests assert *"this plan is
    /// legal"*, and it now means legal all the way down rather than legal until the next layer.
    ///
    /// Use [`anchor_ids`] where the CARDINALITY is the subject.
    fn caller_ids(kind: IdKind) -> StageInput {
        let provenance = if kind == IdKind::Region {
            Some(IdProvenance::Cogmap(CogmapId::new()))
        } else {
            None
        };
        StageInput::Caller {
            relation: StageRelation::Bound,
            ids: IdSet {
                kind,
                provenance,
                ids: vec![uuid::Uuid::now_v7()],
            },
        }
    }

    /// A caller-supplied anchor-kind set of a chosen SIZE — the cardinality is the subject, so it
    /// is a parameter rather than a fixture constant.
    fn anchor_ids(kind: IdKind, n: usize) -> StageInput {
        StageInput::Caller {
            relation: StageRelation::Bound,
            ids: IdSet {
                kind,
                provenance: None,
                ids: (0..n).map(|_| uuid::Uuid::now_v7()).collect(),
            },
        }
    }

    fn caller_ids_no_provenance(kind: IdKind) -> StageInput {
        StageInput::Caller {
            relation: StageRelation::Bound,
            ids: IdSet {
                kind,
                provenance: None,
                ids: vec![],
            },
        }
    }

    fn upstream(name: &str) -> Option<StageInput> {
        Some(StageInput::Upstream {
            relation: StageRelation::Bound,
            stage: StageName::parse(name).unwrap(),
        })
    }

    fn plan_with_property(subject: PropertySubject, key: &str, op: PropertyOp) -> Composition {
        let mut node = act("s", ActName::FollowFrom, Some(caller_ids(IdKind::Resource)));
        if let StageNode::Act(a) = &mut node {
            a.properties.push(PropertyPredicate {
                subject,
                key: key.to_string(),
                op,
            });
        }
        plan(vec![node], vec!["s"])
    }

    // ---- Task 6: topology -------------------------------------------------------------------

    #[test]
    fn a_cycle_is_refused_rather_than_compiled() {
        // A query over a graph must itself be acyclic. Both `follow-from` acts are reachable, so the
        // only thing wrong here is the cycle. (The plan's original used `find-exact`, which Task 8
        // now refuses as unreachable; reachable acts keep the cycle the sole finding.)
        let c = plan(
            vec![
                act("a", ActName::FollowFrom, upstream("b")),
                act("b", ActName::FollowFrom, upstream("a")),
            ],
            vec!["a"],
        );
        let errs = validate(&c).unwrap_err();
        assert!(
            errs.iter().any(|e| e.detail.contains("cycle")),
            "got: {errs:?}"
        );
    }

    #[test]
    fn a_reference_to_an_undeclared_stage_is_refused() {
        let c = plan(
            vec![act("near", ActName::FollowFrom, upstream("ghost"))],
            vec!["near"],
        );
        let errs = validate(&c).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].stage.as_ref().unwrap().as_str(), "near");
    }

    #[test]
    fn two_stages_may_not_share_a_name() {
        let c = plan(
            vec![
                act("hits", ActName::FindExact, None),
                act("hits", ActName::FindAboutAnywhere, None),
            ],
            vec!["hits"],
        );
        assert!(
            validate(&c).is_err(),
            "a duplicate name makes every reference ambiguous"
        );
    }

    #[test]
    fn a_returns_entry_naming_no_stage_is_refused() {
        let c = plan(vec![act("hits", ActName::FindExact, None)], vec!["ghost"]);
        assert!(validate(&c).is_err());
    }

    #[test]
    fn a_combinator_with_one_input_is_refused() {
        // One input is not a combination. Admitting it would let a plan express a no-op node that
        // reads as a merge.
        let c = plan(
            vec![
                act(
                    "hits",
                    ActName::FollowFrom,
                    Some(caller_ids(IdKind::Resource)),
                ),
                StageNode::Combine(CombineNode {
                    name: StageName::parse("both").unwrap(),
                    op: CombineOp::Union,
                    inputs: vec![StageName::parse("hits").unwrap()],
                }),
            ],
            vec!["both"],
        );
        assert!(validate(&c).is_err());
    }

    #[test]
    fn every_refusal_is_reported_not_just_the_first() {
        // A caller repairing a plan should see all of it. Returning the first turns one round trip
        // into N. `follow-from` is reachable, so the two findings are exactly the dangling ref and
        // the bad return — nothing else.
        let c = plan(
            vec![act("near", ActName::FollowFrom, upstream("ghost"))],
            vec!["also_missing"],
        );
        let errs = validate(&c).unwrap_err();
        assert!(
            errs.len() >= 2,
            "expected the dangling ref AND the bad return; got: {errs:?}"
        );
    }

    #[test]
    fn a_valid_plan_comes_back_in_dependency_order() {
        // A `follow-from` chain — both reachable, resource→resource — so the plan is fully legal at
        // the end of beat B and the topological order is the only thing under test.
        let c = plan(
            vec![
                act("near", ActName::FollowFrom, upstream("hits")),
                act(
                    "hits",
                    ActName::FollowFrom,
                    Some(caller_ids(IdKind::Resource)),
                ),
            ],
            vec!["near"],
        );
        let v = validate(&c).expect("plan is legal");
        let names: Vec<&str> = v.ordered().iter().map(|n| n.name().as_str()).collect();
        assert_eq!(
            names,
            vec!["hits", "near"],
            "declaration order is not execution order"
        );
    }

    // ---- Task 7: declaration-driven refusals ------------------------------------------------

    #[test]
    fn a_kind_the_act_does_not_accept_is_refused_against_the_registry() {
        // `find-exact` accepts bounds of kind `resource` only. Piping `survey`'s regions into it is
        // a category error the DECLARATIONS already know about — this check reads them.
        let c = plan(
            vec![
                act("shape", ActName::Survey, Some(caller_ids(IdKind::Cogmap))),
                act("hits", ActName::FindExact, upstream("shape")),
            ],
            vec!["hits"],
        );
        let errs = validate(&c).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::UnsupportedBoundKind),
            "got: {errs:?}"
        );
    }

    /// A cogmap/context bound is served by the fragments' `(table, id)` ANCHOR PAIR, which holds
    /// exactly one id — so a set of any other size is refused, and it is refused HERE.
    ///
    /// `[moved from the compiler — 2026-08-09, Pete]` The compiler was the first thing to notice,
    /// and it noticed by returning `Err`, which aborts the WHOLE composition: a healthy `find-exact`
    /// stage beside a two-context `find-about-within` lost both. That is the same defect the
    /// per-stage embedding refusal removed one layer up, and the repair is not a second runtime
    /// refusal — the cardinality is a STATIC property of the plan, decidable with no database, so
    /// it belongs in the 400 alongside every other refusal.
    #[test]
    fn a_multi_id_anchor_bound_is_refused_here_rather_than_costing_the_whole_composition() {
        // `survey` throughout: it takes cogmap/context bounds and is not a find act, so no
        // intention is threaded and the only thing under test is the cardinality.
        let c = plan(
            vec![
                act("ok", ActName::Survey, Some(anchor_ids(IdKind::Cogmap, 1))),
                act(
                    "scoped",
                    ActName::Survey,
                    Some(anchor_ids(IdKind::Context, 2)),
                ),
            ],
            vec!["ok", "scoped"],
        );
        let errs = validate(&c).unwrap_err();
        let anchor: Vec<&PlanRefusal> = errs
            .iter()
            .filter(|e| e.reason == RefusalReason::AnchorTakesOneId)
            .collect();
        assert_eq!(anchor.len(), 1, "got: {errs:?}");
        assert_eq!(
            anchor[0].stage.as_ref().map(|s| s.as_str()),
            Some("scoped"),
            "the refusal names the stage that earned it, not the composition"
        );
        assert!(
            !errs
                .iter()
                .any(|e| e.stage.as_ref().is_some_and(|s| s.as_str() == "ok")),
            "the innocent stage must not be refused for its neighbour's mistake: {errs:?}"
        );
    }

    /// The boundary, so the check above is about CARDINALITY and not about anchors being unwelcome.
    #[test]
    fn a_single_id_anchor_bound_is_exactly_what_the_slot_holds_and_validates() {
        let c = plan(
            vec![act(
                "scoped",
                ActName::Survey,
                Some(anchor_ids(IdKind::Context, 1)),
            )],
            vec!["scoped"],
        );
        assert!(validate(&c).is_ok(), "got: {:?}", validate(&c).unwrap_err());
    }

    /// **Zero is not "unbounded" here, and that is the trap this arm exists for.**
    ///
    /// For a resource ARRAY, empty means bounded-to-nothing and NULL means unbounded — a
    /// distinction the fragments carry and this surface depends on. An anchor has no such pair: the
    /// slot holds one id or the stage is unscoped, so an empty anchor set is a caller who asked to
    /// scope to a set of contexts and named none. Admitting it would silently drop the scope and
    /// answer the unscoped question instead.
    #[test]
    fn an_empty_anchor_set_is_refused_rather_than_read_as_unscoped() {
        let c = plan(
            vec![act(
                "scoped",
                ActName::Survey,
                Some(anchor_ids(IdKind::Context, 0)),
            )],
            vec!["scoped"],
        );
        let errs = validate(&c).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::AnchorTakesOneId),
            "got: {errs:?}"
        );
    }

    #[test]
    fn survey_declines_limit_because_its_bound_is_a_funnel_width() {
        // `survey` admits `regions` and not `limit`, because `wayfind_region_scores` takes a funnel
        // width and has no rows to limit. A term is never reinterpreted to fit.
        let mut node = act("shape", ActName::Survey, Some(caller_ids(IdKind::Cogmap)));
        if let StageNode::Act(a) = &mut node {
            a.terms.insert(BoundTerm::Limit, 10);
        }
        let errs = validate(&plan(vec![node], vec!["shape"])).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.reason == RefusalReason::BoundTermNotApplicable));
    }

    #[test]
    fn an_edge_filter_on_a_resource_only_act_is_declined_not_ignored() {
        let mut node = act("hits", ActName::FindExact, None);
        if let StageNode::Act(a) = &mut node {
            a.edge_filter = Some(EdgeFilter {
                edge_kinds: vec![EdgeKind::LeadsTo],
                labels: vec![],
            });
        }
        let errs = validate(&plan(vec![node], vec!["hits"])).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.reason == RefusalReason::FilterNotApplicable));
    }

    #[test]
    fn a_find_about_stage_without_a_threaded_intention_refuses_rather_than_substituting() {
        // "I chose not to embed" and "I cannot embed" stay distinguishable.
        //
        // `[strengthened at beat D — 2026-08-08]` This used to pin the MissingIntention refusal
        // SPECIFICALLY — present without an intention, gone with one — rather than full legality,
        // because `find-about-anywhere` was ALSO `NotSeparablyReachable` and so could never be
        // legal whatever the intention said. With `search_wide` mapped to `query_find_wide` that
        // second refusal is gone, and the assertion can be what it always wanted to be: an
        // intention makes this plan VALID, not merely less refused. Asserting `is_ok()` is strictly
        // stronger than asserting one reason's absence, which would also hold if the plan were
        // refused for three other reasons.
        let mut c = plan(
            vec![act("wide", ActName::FindAboutAnywhere, None)],
            vec!["wide"],
        );
        c.intention = None;
        let errs = validate(&c).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.reason == RefusalReason::MissingIntention));

        c.intention = Some(Intention {
            query: "salience".to_string(),
            embedded: true,
        });
        assert!(
            validate(&c).is_ok(),
            "with an intention threaded and its mechanic reachable, the plan is legal; got: {:?}",
            validate(&c).err()
        );
    }

    #[test]
    fn an_unbuilt_act_is_refused_as_not_implemented() {
        // `admit` is the one declared-and-unbuilt act — the anti-act. A plan naming it is refused
        // statically, never attempted, and as `NotImplemented` rather than reachability.
        let errs =
            validate(&plan(vec![act("cold", ActName::Admit, None)], vec!["cold"])).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.reason == RefusalReason::NotImplemented));
    }

    #[test]
    fn a_region_set_without_provenance_is_refused() {
        // Context regions and cogmap regions are both RegionId and are NOT interchangeable — a
        // context region's id 404s at the sole consumer of region ids. Checked here, that is a
        // declined plan; unchecked, it is a rediscovered 404.
        let c = plan(
            vec![act(
                "r",
                ActName::FollowFrom,
                Some(caller_ids_no_provenance(IdKind::Region)),
            )],
            vec!["r"],
        );
        let errs = validate(&c).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.reason == RefusalReason::MissingProvenance));
    }

    #[test]
    fn a_kind_changing_hop_is_expressible_so_the_region_phase_is_not_foreclosed() {
        // Spec §4 requirement 3. No v1 act changes kind, so without this a resource-shaped
        // assumption could pass everything and quietly make region-mediated composition
        // unbuildable. `cogmap in -> region out` is a legal hop TODAY with no SQL change.
        let c = plan(
            vec![act(
                "shape",
                ActName::Survey,
                Some(caller_ids(IdKind::Cogmap)),
            )],
            vec!["shape"],
        );
        let v = validate(&c).expect("cogmap in, region out is a legal hop");
        assert_eq!(v.ordered().len(), 1);
    }

    // ---- Task 7b: the property predicate ----------------------------------------------------

    #[test]
    fn a_content_block_subject_is_refused_because_blocks_are_addressable_not_queryable() {
        // Spec §12: block properties exist so provenance can attach to PART of a resource. That is
        // addressability, a different affordance from being a queryable subject.
        let c = plan_with_property(
            PropertySubject::Other("content_block".to_string()),
            "block_role",
            PropertyOp::HasKey,
        );
        let errs = validate(&c).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.reason == RefusalReason::UnknownFilterValue));
    }

    #[test]
    fn an_empty_property_key_is_refused_rather_than_matching_everything() {
        let c = plan_with_property(PropertySubject::Resource, "", PropertyOp::HasKey);
        assert!(validate(&c).is_err());
    }

    #[test]
    fn contains_with_no_values_is_refused_because_it_narrows_nothing() {
        // An empty list is not "match all" and is not "match none" — it is a caller mistake, and
        // silently treating it as either is the confident-empty failure this contract exists to end.
        let c = plan_with_property(
            PropertySubject::Resource,
            "tags",
            PropertyOp::Contains { values: vec![] },
        );
        assert!(validate(&c).is_err());
    }

    // ---- Task 8: the reachability gap -------------------------------------------------------

    #[test]
    fn a_served_act_this_builder_has_no_fragment_for_refuses_honestly() {
        // `[amended at beat D — 2026-08-08]` The subject was `find-exact`, and this test's own
        // comment predicted it would "go RED at beat D, when the find acts acquire fragments". It
        // did: `search_exact` now maps to `query_find_exact`, so `find-exact` is reachable and no
        // longer refuses.
        //
        // The PROPERTY is unchanged and still needs a subject, so it moves to `substantiate` —
        // `Served`, with a real mechanic (`resource_standing_shape`), for which this builder emits
        // no fragment. `NotImplemented` would be FALSE about it (it is served); the honest refusal
        // is the existence-vs-reachability distinction `BuildState` cannot draw.
        //
        // Re-pointing rather than deleting is the point: had this test simply been removed when it
        // went red, beat D would have retired the only witness that the two refusals stay distinct.
        let errs = validate(&plan(
            vec![act("hits", ActName::Substantiate, None)],
            vec!["hits"],
        ))
        .unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::NotSeparablyReachable),
            "a served act with no fragment must not be reported as unbuilt; got: {errs:?}"
        );
    }

    // ---- The relation moved onto the edge ---------------------------------------------------

    #[test]
    fn asking_an_act_to_reach_beyond_a_set_it_can_only_narrow_within_is_refused() {
        // The negative face of carrying the relation on the wire at all. Across the seven acts
        // `accepts_bounds` and `accepts_seeds` are DISJOINT, so the relation is fully determined
        // by the act and could have been derived — and deriving it is precisely the mistake. A
        // caller writing `seed` against `find-exact` asked to reach BEYOND their set; `find-exact`
        // can only narrow within one. Derived, this would have silently executed the narrowing:
        // their question answered as a different question, with a confident page of results.
        let c = plan_returning(
            vec![act_with_relation(
                "hits",
                ActName::FindExact,
                Some(caller_ids(IdKind::Resource)),
                StageRelation::Seed,
            )],
            vec![ReturnSpec {
                stage: StageName::parse("hits").unwrap(),
                with: vec![],
            }],
        );
        let errs = validate(&c).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::UnsupportedSeedKind),
            "got: {errs:?}"
        );
    }

    #[test]
    fn the_same_stage_with_the_relation_its_act_accepts_is_legal() {
        // The other half of the pair. Without it the test above would also pass if `find-exact`
        // refused every input for some unrelated reason, which would make it evidence of nothing.
        let mut c = plan(
            vec![act_with_relation(
                "hits",
                ActName::FindExact,
                Some(caller_ids(IdKind::Resource)),
                StageRelation::Bound,
            )],
            vec!["hits"],
        );
        c.intention = Some(Intention {
            query: "composable fragments".to_string(),
            embedded: true,
        });
        assert!(validate(&c).is_ok(), "got: {:?}", validate(&c).err());
    }

    #[test]
    fn the_relation_is_read_from_the_edge_rather_than_from_a_sibling_of_the_input() {
        // `[amended — 2026-08-08]` This check used to read `matches!(inv.bounds_mode, Some(Seed))`
        // off the invocation, where the relation was an `Option` whose "required whenever `input`
        // is present" invariant lived only in prose. An input with no relation therefore fell
        // through to `false` and was checked against `accepts_bounds` — a silent narrowing, the
        // exact substitution the refusal above exists to prevent.
        //
        // That state is now unrepresentable rather than merely unreachable, which is why THIS test
        // can only assert the positive: both relations are read, from the edge, for both input
        // sources. The absent case is pinned where it now lives — as a deserialization failure, in
        // `stage::tests::an_input_with_no_declared_relation_does_not_deserialize`.
        let upstream_seed = act_with_relation(
            "near",
            ActName::FollowFrom,
            upstream("hits"),
            StageRelation::Seed,
        );
        let caller_bound = act_with_relation(
            "hits",
            ActName::FollowFrom,
            Some(caller_ids(IdKind::Resource)),
            StageRelation::Bound,
        );
        let StageNode::Act(up) = &upstream_seed else {
            unreachable!()
        };
        let StageNode::Act(ca) = &caller_bound else {
            unreachable!()
        };
        assert_eq!(
            up.input.as_ref().unwrap().relation(),
            StageRelation::Seed,
            "an upstream edge carries its own relation"
        );
        assert_eq!(
            ca.input.as_ref().unwrap().relation(),
            StageRelation::Bound,
            "so does a caller-supplied one"
        );

        // And `follow-from` accepts seeds, not bounds — so the second plan is refused for the
        // relation and the first is not.
        assert!(validate(&plan(vec![caller_bound], vec!["hits"]))
            .unwrap_err()
            .iter()
            .any(|e| e.reason == RefusalReason::UnsupportedBoundKind));
    }

    // ---- Hydration sections -----------------------------------------------------------------

    #[test]
    fn a_section_this_door_does_not_hydrate_is_refused_with_somewhere_to_go() {
        // `body` is a real word in the shared vocabulary, so it deserializes — and is refused
        // here, which is the point of not making it a narrow query-local enum. A refusal that only
        // said no would leave the caller guessing; this one names `show`.
        let c = plan_returning(
            vec![act(
                "near",
                ActName::FollowFrom,
                Some(caller_ids(IdKind::Resource)),
            )],
            vec![ReturnSpec {
                stage: StageName::parse("near").unwrap(),
                with: vec![ResourceSection::Body],
            }],
        );
        let errs = validate(&c).unwrap_err();
        let hit = errs
            .iter()
            .find(|e| e.reason == RefusalReason::SectionNotAvailable)
            .unwrap_or_else(|| panic!("got: {errs:?}"));
        assert_eq!(hit.stage.as_ref().unwrap().as_str(), "near");
        assert!(hit.detail.contains("show"), "got: {}", hit.detail);
        assert!(
            hit.detail.contains("open-meta"),
            "a refusal must name what this door DOES offer; got: {}",
            hit.detail
        );
    }

    #[test]
    fn a_refused_section_comes_back_alongside_every_other_refusal_not_instead_of_them() {
        // THE reason this refusal lives at validation rather than at deserialization. Were `with`
        // a single-member enum, `body` would fail to parse and serde would short-circuit — this
        // caller would learn about their section and never hear about their dangling reference,
        // in a deserializer's vocabulary rather than this contract's.
        let c = plan_returning(
            vec![act("near", ActName::FollowFrom, upstream("ghost"))],
            vec![ReturnSpec {
                stage: StageName::parse("near").unwrap(),
                with: vec![ResourceSection::Body, ResourceSection::Edges],
            }],
        );
        let errs = validate(&c).unwrap_err();
        assert_eq!(
            errs.iter()
                .filter(|e| e.reason == RefusalReason::SectionNotAvailable)
                .count(),
            2,
            "both refused sections, not just the first; got: {errs:?}"
        );
        assert!(
            errs.iter().any(|e| e.detail.contains("undeclared stage")),
            "and the unrelated topology refusal survives alongside them; got: {errs:?}"
        );
    }

    #[test]
    fn the_section_this_door_does_hydrate_passes() {
        // The bite check for the pair above: if `ADMITTED_SECTIONS` were empty, or the containment
        // test were inverted, every one of these tests would still be green.
        let mut c = plan_returning(
            vec![act("wide", ActName::FindAboutAnywhere, None)],
            vec![ReturnSpec {
                stage: StageName::parse("wide").unwrap(),
                with: vec![ResourceSection::OpenMeta],
            }],
        );
        c.intention = Some(Intention {
            query: "salience".to_string(),
            embedded: true,
        });
        assert!(validate(&c).is_ok(), "got: {:?}", validate(&c).err());
    }

    // ---- What validate PROMISES about the response ------------------------------------------

    #[test]
    fn the_outcome_names_the_currency_and_the_score_kind_of_each_returned_stage() {
        // The thing OpenAPI cannot state: `returned` is an open map whose keys are the caller's own
        // stage names and whose value shape depends on which act each stage runs.
        //
        // With the envelope tagging CURRENCY only, `produced` alone no longer separates two acts
        // that both return resources — which is exactly why the promise carries `score_kind` too.
        // Told "resources" plus "vec_norm", a caller can write their parser and stop thinking.
        let mut c = plan(
            vec![
                act("wide", ActName::FindAboutAnywhere, None),
                act("quoted", ActName::FindExact, None),
            ],
            vec!["wide", "quoted"],
        );
        c.intention = Some(Intention {
            query: "composable fragments".to_string(),
            embedded: true,
        });
        let out = ValidationOutcome::of(&validate(&c).expect("plan is legal"));
        let wide = &out.will_return[&StageName::parse("wide").unwrap()];
        let quoted = &out.will_return[&StageName::parse("quoted").unwrap()];

        assert_eq!(wide.produced, ProducedVariant::Resources);
        assert_eq!(quoted.produced, ProducedVariant::Resources);
        assert_eq!(
            wide.produced, quoted.produced,
            "same currency, which the envelope now says and nothing more"
        );
        assert_eq!(wide.score_kind, Some(ScoreKind::VecNorm));
        assert_eq!(quoted.score_kind, Some(ScoreKind::FtsNorm));
        assert_ne!(
            wide.score_kind, quoted.score_kind,
            "and the quantity is what tells them apart"
        );
    }

    #[test]
    fn the_promised_score_kind_and_the_promised_range_name_the_same_quantity() {
        // Two facts about one quantity, and they must not drift: `score_kind` is what a ROW will
        // carry, `orders_by` is where its RANGE lives (which cannot sensibly ride per row). Both
        // derive from `orders_by.field`, and this is what holds them together — a caller reads the
        // kind off a hit and looks up its range on the stage, so a mismatch would send them to the
        // wrong scale.
        let c = plan(
            vec![act(
                "shape",
                ActName::Survey,
                Some(caller_ids(IdKind::Cogmap)),
            )],
            vec!["shape"],
        );
        let out = ValidationOutcome::of(&validate(&c).expect("legal"));
        let shape = &out.will_return[&StageName::parse("shape").unwrap()];

        assert_eq!(shape.produced, ProducedVariant::Regions);
        assert_eq!(
            shape.score_kind.as_ref().map(ScoreKind::as_str),
            shape.orders_by.as_ref().map(|q| q.field.as_str())
        );
        // And the range is carried, because region_score is NOT [0,1] and its name does not say so.
        assert!(matches!(
            shape.orders_by.as_ref().map(|q| &q.scale),
            Some(crate::types::query::QuantityScale::OtherRange { .. })
        ));
    }

    #[test]
    fn the_outcome_carries_exactly_the_keys_returns_asked_for_no_more_no_fewer() {
        // A stage that only FEEDS a downstream one is never hydrated and must not appear here —
        // promising a key the response will not carry is the same class of lie as omitting one it
        // will.
        let mut c = plan(
            vec![
                act("seeds", ActName::FindAboutAnywhere, None),
                act("near", ActName::FollowFrom, upstream("seeds")),
            ],
            vec!["near"],
        );
        c.intention = Some(Intention {
            query: "x".to_string(),
            embedded: true,
        });
        let out = ValidationOutcome::of(&validate(&c).expect("legal"));

        assert_eq!(out.will_return.len(), 1);
        assert!(out
            .will_return
            .contains_key(&StageName::parse("near").unwrap()));
        assert!(
            !out.will_return
                .contains_key(&StageName::parse("seeds").unwrap()),
            "an intermediate stage is not returned, so it is not promised"
        );
        // But it IS in the run order — the trace covers every stage, and the order is the other
        // thing a DAG does not otherwise reveal.
        assert_eq!(
            out.order,
            vec![
                StageName::parse("seeds").unwrap(),
                StageName::parse("near").unwrap()
            ]
        );
    }

    #[test]
    fn the_outcome_tells_a_caller_which_optional_fields_will_be_null() {
        // The reason `discloses` exists at all. `follow-from` cannot report input contribution —
        // its walk discards path origin — and a caller learns that HERE rather than by finding a
        // null in a response and guessing whether it means "cannot" or "none".
        let c = plan(
            vec![act(
                "near",
                ActName::FollowFrom,
                Some(caller_ids(IdKind::Resource)),
            )],
            vec!["near"],
        );
        let out = ValidationOutcome::of(&validate(&c).expect("legal"));
        let near = &out.will_return[&StageName::parse("near").unwrap()];

        assert_eq!(near.produced, ProducedVariant::Resources);
        assert_eq!(near.score_kind, Some(ScoreKind::GraphScore));
        assert!(
            near.discloses.is_empty(),
            "follow-from declares neither disclosure; got: {:?}",
            near.discloses
        );
        // So a caller also knows `located_at` will be absent on every row, before asking.
        assert!(!near
            .discloses
            .contains(&crate::types::query::Disclosure::MatchLocation));
    }

    #[test]
    fn a_returned_combinator_is_refused_rather_than_silently_omitted_from_the_promise() {
        // `[re-pointed — 2026-08-09]` This asserted that a combinator was ABSENT from `will_return`
        // — "an honest 'I cannot tell you' instead of a fabricated variant" — and its own comment
        // noted that a combinator "cannot currently be a returned stage. Whoever makes it one has
        // to come here."
        //
        // The omission was not enough, and review found what it left open. Nothing refused the
        // plan: the compiler emitted a hit arm for the combinator, and the assembler dropped every
        // row for want of a score kind — answering `disposition: answered` with an empty list
        // beside a tally saying rows existed. A promise that silently omits a key is not a
        // constraint; it is a gap the layers below walk straight through.
        //
        // The limitation is now enforced where it is decidable. A combinator's rows come from two
        // or more acts, so the stage has no single `orders_by` and no single `score_kind`, and
        // hydrating it would put two acts' rows into ONE ordered list — the merged list
        // `no-cross-act-ranking` exists to make unrepresentable. Combining stays legal; asking for
        // the combined rows back does not.
        let c = plan(
            vec![
                act("a", ActName::FollowFrom, Some(caller_ids(IdKind::Resource))),
                act("b", ActName::FollowFrom, Some(caller_ids(IdKind::Resource))),
                StageNode::Combine(CombineNode {
                    name: StageName::parse("merged").unwrap(),
                    op: CombineOp::Union,
                    inputs: vec![
                        StageName::parse("a").unwrap(),
                        StageName::parse("b").unwrap(),
                    ],
                }),
            ],
            vec!["merged"],
        );
        let errs = validate(&c).expect_err("a combinator may not be returned");
        assert!(
            errs.iter().any(|e| e.reason
                == RefusalReason::Other("combinator-not-returnable".to_string())
                && e.stage.as_ref().is_some_and(|s| s.as_str() == "merged")),
            "got: {errs:?}"
        );
    }

    #[test]
    fn a_combinator_that_is_not_returned_stays_legal() {
        // The boundary: combining is the point of having combinators. Only asking for the combined
        // rows BACK is refused, and a composition that unions two stages and returns one of them is
        // exactly the shape the DAG exists for.
        let c = plan(
            vec![
                act("a", ActName::FollowFrom, Some(caller_ids(IdKind::Resource))),
                act("b", ActName::FollowFrom, Some(caller_ids(IdKind::Resource))),
                StageNode::Combine(CombineNode {
                    name: StageName::parse("merged").unwrap(),
                    op: CombineOp::Union,
                    inputs: vec![
                        StageName::parse("a").unwrap(),
                        StageName::parse("b").unwrap(),
                    ],
                }),
            ],
            vec!["a"],
        );
        assert!(validate(&c).is_ok(), "got: {:?}", validate(&c).unwrap_err());
    }

    #[test]
    fn the_promised_variant_is_the_one_a_real_output_reports() {
        // The two halves meeting: `ActDeclaration::produced_variant` PREDICTS from a declaration,
        // `StageOutput::variant` REPORTS from an actual output. Separate enums, or one hand-kept
        // table, could disagree — and a caller who parsed against the promise would break on the
        // response.
        use crate::types::query::stage::StageOutput;
        for (variant, actual) in [
            (
                ProducedVariant::Resources,
                StageOutput::Resources { hits: vec![] },
            ),
            (
                ProducedVariant::Regions,
                StageOutput::Regions { hits: vec![] },
            ),
        ] {
            assert_eq!(variant, actual.variant());
        }
    }

    #[test]
    fn the_two_placeholder_fused_acts_are_not_refused_by_reading_their_host() {
        // `follow-from` and `survey` declare `Fused { host: "unified_search" }`, a value registry.rs
        // documents as the least-wrong of three rather than a fact. Their `served_by` mechanics
        // (`search_graph_expand`, `wayfind_region_scores`) are the two the builder calls directly. A
        // rule keyed on the `Fused` discriminant would refuse exactly the two acts beat C exists to
        // execute; this is what makes that inversion fail loudly instead of looking like caution.
        assert!(validate(&plan(
            vec![act(
                "shape",
                ActName::Survey,
                Some(caller_ids(IdKind::Cogmap))
            )],
            vec!["shape"],
        ))
        .is_ok());
        assert!(validate(&plan(
            vec![act(
                "near",
                ActName::FollowFrom,
                Some(caller_ids(IdKind::Resource))
            )],
            vec!["near"],
        ))
        .is_ok());
    }
}
