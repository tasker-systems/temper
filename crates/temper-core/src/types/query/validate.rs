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

use std::collections::{BTreeMap, BTreeSet};

use super::act::{ActName, BuildState};
use super::composition::{Composition, ReturnSpec, StageNode};
use super::disposition::RefusalReason;
use super::envelope::ActInvocation;
use super::filter::{FilterField, PropertyOp, PropertySubject};
use super::id_set::IdKind;
use super::registry::declaration;
use super::scalars::BoundsMode;
use super::stage::{StageInput, StageName};

/// Declared mechanic (`served_by`) → the SQL function the compiler actually emits.
///
/// An act whose `served_by` is absent here is declared and built but not reachable from THIS
/// surface — refused as [`RefusalReason::NotSeparablyReachable`], never as `NotImplemented`.
///
/// A MAP rather than a set since beat D, and the two names differ for the find acts on purpose:
/// `served_by` must keep naming what `/api/search` calls, because that is the mechanic the
/// declaration describes, while `/api/query` emits the composable twin that accepts
/// `p_bound_ids uuid[]`. After `20260808000030` those are the same body — `search_exact` IS
/// `query_find_exact` at NULL bounds — but they are not the same signature, and the declaration
/// describes the door, not the compiler.
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
    ("search_exact", "query_find_exact"),
    ("search_wide", "query_find_wide"),
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRefusal {
    /// The stage it attaches to, when it attaches to one.
    pub stage: Option<StageName>,
    pub reason: RefusalReason,
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

    // Input kind: a bound or a seed, consumed per `bounds_mode`.
    if let Some(input) = &inv.input {
        let (incoming, from_caller, provenance) = match input {
            StageInput::Caller { ids } => (Some(ids.kind.clone()), true, ids.provenance),
            StageInput::Upstream { stage } => {
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
            let as_seed = matches!(inv.bounds_mode, Some(BoundsMode::Seed));
            if as_seed && !decl.accepts_seeds.contains(&kind) {
                errs.push(refusal(
                    Some(name),
                    RefusalReason::UnsupportedSeedKind,
                    format!("act does not accept seeds of kind `{kind:?}`"),
                ));
            } else if !as_seed && !decl.accepts_bounds.contains(&kind) {
                errs.push(refusal(
                    Some(name),
                    RefusalReason::UnsupportedBoundKind,
                    format!("act does not accept bounds of kind `{kind:?}`"),
                ));
            }
        }
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

    // A find-about-* stage refuses without a threaded intention rather than embedding on the
    // caller's behalf — "I chose not to embed" and "I cannot embed" stay distinct.
    if matches!(
        inv.act,
        ActName::FindAboutAnywhere | ActName::FindAboutWithin
    ) && c.intention.is_none()
    {
        errs.push(refusal(
            Some(name),
            RefusalReason::MissingIntention,
            "a find-about-* stage requires a threaded intention",
        ));
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

    // A `returns` entry must name a declared stage.
    for ret in &c.outcome.returns {
        if !declared.contains(ret.stage.as_str()) {
            errs.push(refusal(
                Some(&ret.stage),
                RefusalReason::Other("unknown-return-stage".to_string()),
                format!("returns names undeclared stage `{}`", ret.stage.as_str()),
            ));
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
    use crate::types::query::disposition::RefusalDisposition;
    use crate::types::query::envelope::ActInvocation;
    use crate::types::query::filter::{EdgeFilter, PropertyPredicate};
    use crate::types::query::id_set::{IdKind, IdProvenance, IdSet};
    use crate::types::query::scalars::BoundTerm;
    use std::collections::BTreeMap;

    /// A minimal legal act node. `bounds_mode` follows the act: `follow-from` seeds, everything else
    /// bounds — chosen so the input-kind check reads the right acceptance list.
    fn act(name: &str, a: ActName, input: Option<StageInput>) -> StageNode {
        let bounds_mode = input.as_ref().map(|_| {
            if a == ActName::FollowFrom {
                BoundsMode::Seed
            } else {
                BoundsMode::Bound
            }
        });
        StageNode::Act(ActInvocation {
            name: StageName::parse(name).unwrap(),
            act: a,
            input,
            bounds_mode,
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    }

    fn plan(stages: Vec<StageNode>, returns: Vec<&str>) -> Composition {
        Composition {
            outcome: OutcomeDeclaration {
                description: "test plan".to_string(),
                returns: returns
                    .into_iter()
                    .map(|s| ReturnSpec {
                        stage: StageName::parse(s).unwrap(),
                        fields: vec![],
                    })
                    .collect(),
            },
            intention: None,
            on_stage_refusal: RefusalDisposition::Halt,
            meta_detail: Default::default(),
            bounds: BTreeMap::new(),
            stages,
        }
    }

    /// A caller id set, with region provenance supplied so it is well-formed.
    fn caller_ids(kind: IdKind) -> StageInput {
        let provenance = if kind == IdKind::Region {
            Some(IdProvenance::Cogmap(CogmapId::new()))
        } else {
            None
        };
        StageInput::Caller {
            ids: IdSet {
                kind,
                provenance,
                ids: vec![],
            },
        }
    }

    fn caller_ids_no_provenance(kind: IdKind) -> StageInput {
        StageInput::Caller {
            ids: IdSet {
                kind,
                provenance: None,
                ids: vec![],
            },
        }
    }

    fn upstream(name: &str) -> Option<StageInput> {
        Some(StageInput::Upstream {
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
