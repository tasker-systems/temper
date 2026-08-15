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
//!
//! **It answers two different questions, and they are two modules.** `shape` asks whether the
//! composition is *expressible* — true of the plan and the published wire contract alone, so it
//! cannot change under a caller's feet. `capability` asks whether THIS server has built the
//! thing, which moves with every beat. [`validate`] is both; [`validate_shape`] is the first
//! alone, which is what a client may run locally against a server whose binary it does not share.

mod capability;
mod shape;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::act::{ActName, ActQuantity, Disclosure};
use super::composition::{Composition, ReturnSpec, StageNode};
use super::disposition::RefusalReason;
use super::hits::ScoreKind;
use super::registry::declaration;
use super::scalars::BoundTerm;
use super::stage::{ProducedVariant, StageName};

/// Declared mechanic (`served_by`) → the SQL function the compiler actually emits.
///
/// An act whose `served_by` is absent here is declared and built but not reachable from THIS
/// surface — refused as [`RefusalReason::NotSeparablyReachable`], never as `NotImplemented`.
///
/// A MAP rather than a set since beat D, and the key and value still differ for the find acts —
/// but `[corrected — 2026-08-12]` NOT for the reason this comment used to give. It said `served_by`
/// "must keep naming what `/api/search` calls … while `/api/query` emits the fragment that accepts
/// `p_bound_ids uuid[]`", which put the gated twins on one side of the split and the door on the
/// other. That distinction is gone: `/api/search` gained a resource bound, its read path was
/// repointed, and the twin is now exactly what the deployed door calls — so `served_by` names
/// `query_find_exact` and this map is keyed on it.
///
/// **What the split IS, and it is the durable one: GATED entry point → UNGATED core.** The values
/// are the ungated cores (`20260808000030`), never the twins. `/api/query` hoists the visibility
/// relation into ONE CTE and hands it to every stage, which is only possible against a fragment
/// that does not gate internally — a twin per stage would recompute `resources_visible_to` N times,
/// since the planner does not dedupe it across call sites. `/api/search` runs one stage and so
/// enters at the gated twin. Still ONE BODY per arm: `search_exact` → `query_find_exact` →
/// `__temper_ungated_find_exact`, three signatures over one implementation.
/// A caller of this map must therefore already hold an RBAC verdict — in this crate the map only
/// decides reachability, and the sole emitter of these names is
/// `temper_substrate::readback::query_plan::emit_ungated_core_call`, which supplies that verdict
/// itself rather than taking it as an argument.
///
/// Membership is what decides `NotSeparablyReachable`, and it is keyed on served-by names and NEVER
/// on `build_state`. What holds that rule today is `substantiate`: `Served`, with a real mechanic
/// (`resource_standing_shape`), and absent here — so a rule keyed on the discriminant would admit
/// an act this surface cannot emit. **The `Fused` half of that argument no longer has a witness**:
/// since `follow-from` and `survey` left this map, the acts absent from it are `substantiate`,
/// `follow-from` and `survey` — all three unreachable, and the latter two exactly the `Fused` ones —
/// so keying on that discriminant would agree with this map about those two by coincidence. One
/// direction, said as one direction.
///
/// **`survey` is ABSENT** rather than mapped to the deliberately-absent placeholder
/// (`__temper_unbound_act`), which it carried through beat C. Mapped, it validated clean and then
/// failed at EXECUTION — invisible while nothing executed a composition outside its own tests, and
/// a 500 the moment a door opened. Absent, it refuses statically as
/// [`RefusalReason::NotSeparablyReachable`], which is what keeps `registry.rs`'s `DoorReach::Absent`
/// TRUE for it at all three doors: mapped, both `Absent` and its promised restoration to `Serves`
/// would have been false at once — reachable through the door, and unable to answer.
///
/// It cannot simply be wired up: `wayfind_region_scores` takes a `p_lens` no slot supplies.
///
/// `[joined — 2026-08-14]` **`follow-from` left that company.** It was absent for the same reason —
/// `search_graph_expand` takes `p_depth`/`p_gamma` no slot supplies — and the answer was not a slot
/// for either. Both are DEFINITIONAL rather than caller inputs (the act fixes depth at 2 and gamma
/// at the rate its `orders_by` sentence describes), so `query_follow_from` takes them and the
/// compiler passes constants. What the act needed a slot for was the seed set it already had, and
/// the bound it did not — `20260814000030` plus `inputs: Vec<StageInput>`.
///
/// `[added — 2026-08-14]` `find-resources-with` joins as the third member. It is the first entry
/// whose fragment takes NO intention and returns NO quantity — the map says nothing about either,
/// which is why it needed no widening to admit one.
const CALLABLE_FRAGMENTS: &[(&str, &str)] = &[
    ("query_find_exact", "__temper_ungated_find_exact"),
    (
        "query_find_resources_with",
        "__temper_ungated_find_resources_with",
    ),
    ("query_find_wide", "__temper_ungated_find_wide"),
    ("query_follow_from", "__temper_ungated_follow_from"),
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

/// A bound term as the caller wrote it, for a refusal to quote back.
///
/// Same reason and same mechanism as [`act_wire_name`], and the omission was the same shape: three
/// refusal details rendered `{term:?}`, so a caller who sent `"regions": 200` was told about *the
/// `Regions` bound term*, greps their own request for `Regions`, and finds nothing. `BoundTerm`
/// carries `rename_all = "snake_case"`, so the Debug spelling is never the wire spelling for any
/// member. One naming convention, held by going through serde rather than by remembering.
fn term_wire_name(term: &BoundTerm) -> String {
    serde_json::to_string(term)
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|_| format!("{term:?}"))
}

/// The declared stages indexed by name, **first wins**.
///
/// One definition, called by both passes. A duplicate name is refused by [`shape`], but the two
/// passes still have to agree about which of the two nodes they are talking about while they say
/// so — and two inline builds are two places for that to diverge.
fn index_by_name(c: &Composition) -> BTreeMap<&str, &StageNode> {
    let mut by_name: BTreeMap<&str, &StageNode> = BTreeMap::new();
    for node in &c.stages {
        by_name.entry(node.name().as_str()).or_insert(node);
    }
    by_name
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
    /// Read it and you know in advance whether asking for `located_at` is worth doing — rather
    /// than discovering a null in a response and having to guess whether it means *cannot* or
    /// *none*.
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

/// Validate a composition against `search_family()` and the plan's own topology. Returns ALL
/// refusals, never just the first.
pub fn validate(c: &Composition) -> Result<ValidatedComposition, Vec<PlanRefusal>> {
    let (mut errs, ordered) = shape::validate_shape_indexed(c);

    // The capability pass has two halves and only its per-stage half is gated on the topology.
    // This one reads no stage graph — it compares each `returns` entry's `with` against a
    // constant — so a cycle takes nothing away from its answer, and gating it would drop a
    // refusal a cyclic plan used to receive. Every refusal, not the first, is this module's rule.
    capability::validate_returns(c, &mut errs);

    // The per-stage half runs only when the DAG is acyclic, which is the incumbent behaviour and
    // exactly what the pinned rule is about — `reachable ACTS keep the cycle the sole finding`.
    // Per-stage findings over a graph that is not a graph would be findings about a plan that
    // cannot be read.
    if let Some(ordered) = ordered {
        let by_name = index_by_name(c);
        capability::validate_stages(c, &by_name, &mut errs);
        if errs.is_empty() {
            return Ok(ValidatedComposition {
                composition: c.clone(),
                ordered,
            });
        }
    }

    Err(errs)
}

/// Expressibility alone — every refusal that is true of the plan and the published contract
/// without consulting what this server has built. It exists for a local `--check`: a client may
/// run this against a plan it will send to a server whose binary it does not share.
///
/// `[shipped — 2026-08-13]` This said *"No such command ships yet; PR C adds `temper query --check`
/// as its first caller."* It has two callers now, and the pair is the point: `temper query --check`
/// runs it **alone**, and `temper_services::backend::query_read::prepare` runs it as the gate
/// before embedding. (Named, not linked — that crate depends on this one, so a link would point up
/// the dependency graph.) One function, so a plan checked locally and a plan checked by the server
/// cannot disagree about what "well-formed" means.
///
/// **What it still cannot see is capability** — spec §C: *"it reports expressibility and says so —
/// it cannot speak to what the server has implemented and does not try."* That is why `--check`
/// ships a disclosure rather than a verdict: a plan naming an act this deployment has not made
/// reachable is shape-clean here and refused there.
pub fn validate_shape(c: &Composition) -> Vec<PlanRefusal> {
    shape::validate_shape_indexed(c).0
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
    use crate::types::query::filter::{
        EdgeFilter, FacetPredicate, PropertyOp, PropertyPredicate, PropertySubject, ResourceFilter,
    };
    use crate::types::query::id_set::{IdKind, IdProvenance, IdSet};
    use crate::types::query::scalars::BoundTerm;
    use uuid::Uuid;
    // Named here rather than reached through `super::*`: the checks that read them moved into
    // `shape` and `capability`, so this module no longer imports them for its own use.
    use crate::types::query::stage::{StageInput, StageRelation};
    use crate::types::resource_view::ResourceSection;
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

    /// A stage carrying MORE than one input — the shape `inputs: Vec<StageInput>` exists for.
    fn act_with_inputs(name: &str, a: ActName, inputs: Vec<StageInput>) -> StageNode {
        let intention = natural_intention(&a);
        StageNode::Act(ActInvocation {
            name: StageName::parse(name).unwrap(),
            act: a,
            intention,
            inputs,
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    }

    /// **Two inputs in the SAME relation is malformed, and is refused rather than unioned.**
    ///
    /// `[added — 2026-08-14]` with the widening. Merging them is `CombineOp::Union` — a stage the
    /// caller declares, that shows up in the trace with its own `produced` count. Doing it inside
    /// one act's input list would be the same merge with no stage, no tally and nothing saying so.
    #[test]
    fn two_inputs_in_the_same_relation_are_refused_rather_than_merged() {
        let ids = |n: usize| IdSet {
            kind: IdKind::Resource,
            provenance: None,
            ids: (0..n).map(|_| Uuid::now_v7()).collect(),
        };
        let node = act_with_inputs(
            "hits",
            ActName::FindExact,
            vec![
                StageInput::Caller {
                    relation: StageRelation::Bound,
                    ids: ids(2),
                },
                StageInput::Caller {
                    relation: StageRelation::Bound,
                    ids: ids(3),
                },
            ],
        );
        let result = validate(&plan(vec![node], vec!["hits"]));
        let refusals = result.expect_err("two bounds on one stage is malformed");
        assert!(
            refusals
                .iter()
                .any(|e| e.reason == RefusalReason::DuplicateInputRelation),
            "expected `duplicate_input_relation`; got {refusals:?}"
        );
    }

    /// **One seed and one bound on the same stage is WELL-FORMED**, which is the whole point of the
    /// widening — and the assertion that keeps the duplicate check above from being written as
    /// "at most one input" by accident.
    #[test]
    fn a_seed_and_a_bound_on_one_stage_are_not_a_duplicate() {
        let ids = IdSet {
            kind: IdKind::Resource,
            provenance: None,
            ids: vec![Uuid::now_v7()],
        };
        let node = act_with_inputs(
            "walk",
            ActName::FollowFrom,
            vec![
                StageInput::Caller {
                    relation: StageRelation::Seed,
                    ids: ids.clone(),
                },
                StageInput::Caller {
                    relation: StageRelation::Bound,
                    ids,
                },
            ],
        );
        let result = validate(&plan(vec![node], vec!["walk"]));
        // `follow-from` is still refused for other reasons — it is absent from `CALLABLE_FRAGMENTS`
        // and declares `accepts_bounds: []` — so this asserts the ABSENCE of the duplicate refusal
        // rather than success. Naming the reason is what makes it survive those other refusals
        // retiring.
        if let Err(refusals) = result {
            assert!(
                !refusals
                    .iter()
                    .any(|e| e.reason == RefusalReason::DuplicateInputRelation),
                "a seed and a bound are two relations, not a duplicate; got {refusals:?}"
            );
        }
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
        let intention = natural_intention(&a);
        StageNode::Act(ActInvocation {
            name: StageName::parse(name).unwrap(),
            act: a,
            intention,
            inputs: input.into_iter().collect(),
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    }

    /// The intention an act needs to be well-formed: a question for the find acts, nothing for the
    /// rest. The sibling of [`natural_relation`], and it exists for the same reason — a helper that
    /// built every act WITHOUT one would make `MissingIntention` join the finding in every test
    /// about something else, exactly as a wrong relation would.
    ///
    /// `[2026-08-12]` Replaces the `plan_with_intention` helper, which threaded one question onto
    /// the composition for the whole plan. Spec ⟨7⟩ moved the field onto the stage, so supplying it
    /// is now the act builder's job.
    fn natural_intention(a: &ActName) -> Option<Intention> {
        matches!(
            a,
            ActName::FindExact | ActName::FindAboutAnywhere | ActName::FindAboutWithin
        )
        .then(|| Intention {
            query: "q".to_string(),
            embedding: None,
        })
    }

    /// A find act carrying NO question — for the tests that assert `MissingIntention`. Deliberately
    /// separate from [`act`], so a test that wants the refusal says so in the builder it calls
    /// rather than by mutating a plan after the fact.
    #[allow(dead_code)]
    fn act_without_intention(name: &str, a: ActName, input: Option<StageInput>) -> StageNode {
        match act(name, a, input) {
            StageNode::Act(mut inv) => {
                inv.intention = None;
                StageNode::Act(inv)
            }
            other => other,
        }
    }

    /// Strip the question from every act in a plan.
    ///
    /// `[2026-08-12]` Replaces `c.intention = None`, which was one assignment because there was one
    /// intention. **These two helpers are a translation of the old shape, and they flatten a
    /// distinction that now exists:** with the question per stage, a plan may have SOME stages
    /// carrying one, which is neither "threaded" nor "absent" and is what the new tests in
    /// `shape.rs` cover directly. Used here so the incumbent tests keep asserting what they always
    /// asserted, with no silent change of subject.
    fn clear_intentions(c: &mut Composition) {
        for node in &mut c.stages {
            if let StageNode::Act(inv) = node {
                inv.intention = None;
            }
        }
    }

    /// Give every act in a plan the same question — the old envelope semantics, stated explicitly
    /// rather than implied by the type. See [`clear_intentions`] for why these exist.
    fn set_intentions(c: &mut Composition, query: &str) {
        for node in &mut c.stages {
            if let StageNode::Act(inv) = node {
                inv.intention = Some(Intention {
                    query: query.to_string(),
                    embedding: None,
                });
            }
        }
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

    /// `[retained as an alias — 2026-08-12]` Was [`plan`] plus a composition-level question. Spec
    /// ⟨7⟩ moved the question onto each stage, where [`act`] now supplies it, so the two are the
    /// same function. Kept rather than sed-ed away at ~11 call sites because the NAME still says
    /// something true about those tests — they are the ones whose acts need a question — and a
    /// mechanical rename would bury that in the diff of a PR that is already large.
    fn plan_with_intention(stages: Vec<StageNode>, returns: Vec<&str>) -> Composition {
        plan(stages, returns)
    }

    fn plan_returning(stages: Vec<StageNode>, returns: Vec<ReturnSpec>) -> Composition {
        Composition {
            outcome: OutcomeDeclaration { returns },
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

    /// One stage over `a`, a threaded question, and a `returns` naming that stage — a composition
    /// with nothing in it but the minimum a plan needs to be well-formed. No input, no terms, no
    /// filters, no predicates, so nothing here can raise a refusal of its own and obscure the one
    /// under test.
    fn a_legal_single_stage_plan_over(a: ActName) -> Composition {
        plan_with_intention(vec![act("s", a, None)], vec!["s"])
    }

    fn plan_with_property(subject: PropertySubject, key: &str, op: PropertyOp) -> Composition {
        let mut node = act("s", ActName::FindExact, Some(caller_ids(IdKind::Resource)));
        if let StageNode::Act(a) = &mut node {
            a.properties.push(PropertyPredicate {
                subject,
                key: key.to_string(),
                op,
            });
        }
        plan_with_intention(vec![node], vec!["s"])
    }

    // ---- Task 6: topology -------------------------------------------------------------------

    #[test]
    fn a_cycle_is_refused_rather_than_compiled() {
        // A query over a graph must itself be acyclic. Both `find-exact` acts are reachable, so the
        // only thing wrong here is the cycle.
        //
        // The subject has now made a round trip, which is worth recording rather than quietly
        // arriving back where it started: this was `find-exact`, moved to `follow-from` at beat B
        // because `search_exact` was then absent from `CALLABLE_FRAGMENTS`, and moved back when the
        // two placeholder rows were dropped and `follow-from` became the unreachable one. The rule
        // that decided it both times is the same — a cycle test needs a REACHABLE act, or the
        // reachability refusal joins the finding and `is_err()` stops meaning "the cycle".
        let c = plan_with_intention(
            vec![
                act("a", ActName::FindExact, upstream("b")),
                act("b", ActName::FindExact, upstream("a")),
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
        // A reachable act and a threaded question, so the dangling reference is the ONE refusal —
        // which is what the exact count below is asserting.
        let c = plan_with_intention(
            vec![act("hits", ActName::FindExact, upstream("ghost"))],
            vec!["hits"],
        );
        let errs = validate(&c).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].stage.as_ref().unwrap().as_str(), "hits");
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
        //
        // A reachable act, so the act contributes no refusal of its own. Note what this `is_err()`
        // still cannot separate, which predates the reachability flip: naming a combinator in
        // `returns` is itself refused (`CombinatorNotReturnable`), so the arity check is not the
        // only thing holding this assertion up.
        let c = plan_with_intention(
            vec![
                act(
                    "hits",
                    ActName::FindExact,
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
        // into N. `find-exact` is reachable and the question is threaded, so the two findings are
        // exactly the dangling ref and the bad return — nothing else.
        let c = plan_with_intention(
            vec![act("hits", ActName::FindExact, upstream("ghost"))],
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
        // `find-exact` → `find-about-within`: two reachable acts, resource→resource, and
        // `find-about-within` accepts a resource bound — so the plan is fully legal and the
        // topological order is the only thing under test.
        let c = plan_with_intention(
            vec![
                act("narrowed", ActName::FindAboutWithin, upstream("hits")),
                act(
                    "hits",
                    ActName::FindExact,
                    Some(caller_ids(IdKind::Resource)),
                ),
            ],
            vec!["narrowed"],
        );
        let v = validate(&c).expect("plan is legal");
        let names: Vec<&str> = v.ordered().iter().map(|n| n.name().as_str()).collect();
        assert_eq!(
            names,
            vec!["hits", "narrowed"],
            "declaration order is not execution order"
        );
    }

    // ---- Task 7: declaration-driven refusals ------------------------------------------------

    #[test]
    fn a_kind_the_act_does_not_accept_is_refused_against_the_registry() {
        // `find-exact` does not accept bounds of kind `region`. Piping `survey`'s regions into it is
        // a category error the DECLARATIONS already know about — this check reads them.
        //
        // `survey` stays the upstream because it is the only act that PRODUCES a kind no reachable
        // act accepts; the alternative subjects are all resource-to-resource. Its own
        // `NotSeparablyReachable` rides along, which is why the assertion names the reason it means.
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
        // `find-exact` throughout: it is reachable and declares `accepts_bounds: [Resource,
        // Context, Cogmap]`, so with the question threaded the only thing under test is the
        // cardinality. A reachable act is what the `ok` assertion below needs — an unreachable one
        // would earn a refusal of its own and the innocent stage would no longer be innocent.
        let c = plan_with_intention(
            vec![
                act(
                    "ok",
                    ActName::FindExact,
                    Some(anchor_ids(IdKind::Cogmap, 1)),
                ),
                act(
                    "scoped",
                    ActName::FindExact,
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
        let c = plan_with_intention(
            vec![act(
                "scoped",
                ActName::FindExact,
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
        let c = plan_with_intention(
            vec![act(
                "scoped",
                ActName::FindExact,
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

    /// **The two anchor arms sit on opposite sides of the seam, and only this pins which.**
    ///
    /// `[added — 2026-08-12, Pete's ruling]` `AnchorTakesOneId` is raised from both modules, so the
    /// seam guard — which counts per module and sees ONE site in each — cannot tell whether the
    /// arms are the right way round. The classification is the ruling: naming nothing to scope to
    /// is malformed against every door there will ever be, while "the pair holds one id" is a fact
    /// about TODAY's fragments that an `anchor_ids uuid[]` retires. A client running
    /// `validate_shape` against a newer server must therefore still refuse the empty set and must
    /// NOT refuse the many-id one.
    ///
    /// Both remain refusals of `validate`, which the two tests above assert; what moves is only
    /// which pass raises them.
    #[test]
    fn shape_refuses_an_empty_anchor_and_leaves_the_multi_id_one_to_capability() {
        let empty = plan_with_intention(
            vec![act(
                "scoped",
                ActName::FindExact,
                Some(anchor_ids(IdKind::Context, 0)),
            )],
            vec!["scoped"],
        );
        assert!(
            validate_shape(&empty)
                .iter()
                .any(|e| e.reason == RefusalReason::AnchorTakesOneId),
            "an anchor naming nothing is malformed whatever the fragments hold, so the shape \
             pass must raise it: {:?}",
            validate_shape(&empty)
        );

        let many = plan_with_intention(
            vec![act(
                "scoped",
                ActName::FindExact,
                Some(anchor_ids(IdKind::Context, 2)),
            )],
            vec!["scoped"],
        );
        assert!(
            !validate_shape(&many)
                .iter()
                .any(|e| e.reason == RefusalReason::AnchorTakesOneId),
            "the one-id anchor slot is this door's fragment shape, not the contract's — a stale \
             client must not refuse a plan a widened server would run: {:?}",
            validate_shape(&many)
        );
        assert!(
            validate(&many)
                .unwrap_err()
                .iter()
                .any(|e| e.reason == RefusalReason::AnchorTakesOneId),
            "and the full validator must still refuse it against THIS server"
        );
    }

    /// A negative paging term is the CALLER's error and is refused here — not carried into the
    /// statement to come back as a 500.
    #[test]
    fn a_negative_paging_term_is_refused_rather_than_handed_to_postgres() {
        for term in [BoundTerm::Limit, BoundTerm::Offset] {
            let mut node = act("hits", ActName::FindExact, None);
            if let StageNode::Act(a) = &mut node {
                a.terms.insert(term, -1);
            }
            let c = plan_with_intention(vec![node], vec!["hits"]);
            let errs = validate(&c).unwrap_err();
            assert!(
                errs.iter()
                    .any(|e| e.reason == RefusalReason::BoundTermNotApplicable),
                "{term:?} = -1 must refuse; got: {errs:?}"
            );
        }
    }

    /// The fragments' paging slots are `int`. A value above that range is not a large page — it is
    /// one the mechanic cannot express, and clamping it silently would answer a different question.
    #[test]
    fn a_paging_term_beyond_the_slots_range_is_refused_rather_than_truncated() {
        let mut node = act("hits", ActName::FindExact, None);
        if let StageNode::Act(a) = &mut node {
            a.terms.insert(BoundTerm::Offset, 3_000_000_000);
        }
        let c = plan_with_intention(vec![node], vec!["hits"]);
        let errs = validate(&c).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.reason == RefusalReason::BoundTermNotApplicable));
    }

    /// A stage answers once. Two `returns` entries naming it duplicated every row while its tally
    /// still said one, and only the LAST entry's `with` survived.
    #[test]
    fn a_stage_named_twice_in_returns_is_refused_rather_than_answered_twice() {
        let c = Composition {
            outcome: OutcomeDeclaration {
                returns: vec![
                    ReturnSpec {
                        stage: StageName::parse("hits").unwrap(),
                        with: vec![],
                    },
                    ReturnSpec {
                        stage: StageName::parse("hits").unwrap(),
                        with: vec![ResourceSection::OpenMeta],
                    },
                ],
            },
            stages: vec![act("hits", ActName::FindExact, None)],
        };
        let errs = validate(&c).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::DuplicateReturnStage),
            "got: {errs:?}"
        );
    }

    /// A composition with no stages asks nothing, and the compiler's fallback should never be the
    /// thing that catches it.
    #[test]
    fn a_composition_with_no_stages_is_refused() {
        let c = Composition {
            outcome: OutcomeDeclaration { returns: vec![] },
            stages: vec![],
        };
        let errs = validate(&c).unwrap_err();
        assert!(
            errs.iter().any(|e| e.reason == RefusalReason::NoStages),
            "got: {errs:?}"
        );
    }

    /// A composition with no returns answers nothing — the contract's `returns: minItems 1`,
    /// enforced rather than assumed (audit finding F6: an empty `returns` compiled and ran,
    /// answering 200 where the contract says 400).
    #[test]
    fn a_composition_with_no_returns_is_refused() {
        let c = plan_with_intention(
            vec![act(
                "hits",
                ActName::FindExact,
                Some(caller_ids(IdKind::Resource)),
            )],
            vec![],
        );
        let errs = validate(&c).unwrap_err();
        let no_returns = RefusalReason::NoReturns;
        assert!(
            errs.iter()
                .any(|e| e.reason == no_returns && e.stage.is_none()),
            "composition-level, like no-stages — the omission belongs to no stage; got: {errs:?}"
        );

        // And a one-return composition does not get it — the refusal is about the empty list, not
        // about returns generally.
        let ok = plan_with_intention(
            vec![act(
                "hits",
                ActName::FindExact,
                Some(caller_ids(IdKind::Resource)),
            )],
            vec!["hits"],
        );
        assert!(validate(&ok).is_ok(), "got: {:?}", validate(&ok).err());
    }

    #[test]
    fn survey_declines_limit_because_its_bound_is_a_funnel_width() {
        // `survey` admits `regions` and not `limit`, because `wayfind_region_scores` takes a funnel
        // width and has no rows to limit. A term is never reinterpreted to fit.
        //
        // Kept over `survey` because survey is the SUBJECT — no reachable act declines `limit`. The
        // plan therefore also earns a `NotSeparablyReachable`, which is why the assertion names its
        // reason rather than reading `is_err()`.
        let mut node = act("shape", ActName::Survey, Some(caller_ids(IdKind::Cogmap)));
        if let StageNode::Act(a) = &mut node {
            a.terms.insert(BoundTerm::Limit, 10);
        }
        let errs = validate(&plan(vec![node], vec!["shape"])).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.reason == RefusalReason::BoundTermNotApplicable));
    }

    /// **Every narrowing this door drops is refused — all nine, not the one that had a test.**
    ///
    /// A narrowing is "declined, never ignored": accepting one the compiler emits no slot for
    /// answers a different question than the one asked, and then echoes the filter back in
    /// `narrowed_by` as evidence for the answer.
    ///
    /// Written as one parameterised case per field because the rule is about the SET. Eight of the
    /// nine arms had no witness while `tags` had one, so restricting the rule to `tags` — or
    /// dropping any single arm — survived the whole suite. A test per field would have the same
    /// hole the moment a tenth narrowing is added; this one at least fails loudly when an existing
    /// arm goes missing.
    ///
    /// Presence, not exclusivity: `stage` / `status` / `doc_type` also carry closed vocabularies, so
    /// a case may legitimately raise `UnknownFilterValue` beside the refusal under test. The
    /// validator returns ALL refusals, so asserting the one this test is about stays sharp.
    #[test]
    fn every_narrowing_this_door_cannot_apply_is_refused_and_none_is_silently_dropped() {
        /// One case: a label, the narrowing the caller supplied, and the fragment of the refusal
        /// that must name it back.
        type NarrowingCase = (&'static str, Box<dyn Fn(&mut ActInvocation)>, &'static str);

        let cases: Vec<NarrowingCase> = vec![
            (
                "tags",
                Box::new(|a: &mut ActInvocation| {
                    a.resource_filter = Some(ResourceFilter {
                        tags: vec!["x".to_string()],
                        ..Default::default()
                    })
                }),
                "narrow by `tags`",
            ),
            (
                "facets",
                Box::new(|a: &mut ActInvocation| {
                    a.resource_filter = Some(ResourceFilter {
                        facets: vec![FacetPredicate {
                            key: "k".to_string(),
                            value: "v".to_string(),
                        }],
                        ..Default::default()
                    })
                }),
                "narrow by `facets`",
            ),
            (
                "stage",
                Box::new(|a: &mut ActInvocation| {
                    a.resource_filter = Some(ResourceFilter {
                        stage: Some("done".to_string()),
                        ..Default::default()
                    })
                }),
                "narrow by `stage`",
            ),
            (
                "status",
                Box::new(|a: &mut ActInvocation| {
                    a.resource_filter = Some(ResourceFilter {
                        status: Some("active".to_string()),
                        ..Default::default()
                    })
                }),
                "narrow by `status`",
            ),
            (
                "owner",
                Box::new(|a: &mut ActInvocation| {
                    a.resource_filter = Some(ResourceFilter {
                        owner: Some("@someone".to_string()),
                        ..Default::default()
                    })
                }),
                "narrow by `owner`",
            ),
            (
                "title_contains",
                Box::new(|a: &mut ActInvocation| {
                    a.resource_filter = Some(ResourceFilter {
                        title_contains: Some("pipe".to_string()),
                        ..Default::default()
                    })
                }),
                "narrow by `title_contains`",
            ),
            (
                // `[changed — 2026-08-14]` Was "two doc types", expecting *"holds exactly one
                // value"* — a refusal about the FRAGMENT's single `text` slot, which admitted one
                // value and refused several. `doc_type` is no longer a modifier at all, so the case
                // is now ONE value and the refusal is about the field rather than its arity. Kept as
                // a changed case rather than a new one beside a deleted one, so the diff shows a
                // refusal that widened and not a witness that vanished.
                "doc_type",
                Box::new(|a: &mut ActInvocation| {
                    a.resource_filter = Some(ResourceFilter {
                        doc_type: vec!["task".to_string()],
                        ..Default::default()
                    })
                }),
                "narrow by `doc_type`",
            ),
            (
                "a property predicate",
                Box::new(|a: &mut ActInvocation| {
                    a.properties = vec![PropertyPredicate {
                        subject: PropertySubject::Resource,
                        key: "k".to_string(),
                        op: PropertyOp::HasKey,
                    }]
                }),
                "property predicates",
            ),
            (
                // `[changed — 2026-08-14]` The refusal moved. It came from an UNCONDITIONAL site
                // ("this door does not yet apply edge filters"), which is retired now that
                // `follow-from` compiles to a fragment carrying both of `EdgeFilter`'s axes. What
                // refuses here is the PER-ACT check, which was dead code while the unconditional
                // one ran first — so this case now witnesses a different, narrower rule: a find act
                // does not traverse an edge, so it cannot filter one.
                "an edge filter",
                Box::new(|a: &mut ActInvocation| {
                    a.edge_filter = Some(EdgeFilter {
                        edge_kinds: vec![EdgeKind::LeadsTo],
                        labels: vec![],
                    })
                }),
                "cannot apply edge filters",
            ),
        ];

        // The denominator, stated so a shrinking case list is a failure rather than a quieter test.
        assert_eq!(cases.len(), 9, "nine narrowings a find act drops");

        for (label, supply, expected) in cases {
            let mut node = act("hits", ActName::FindExact, None);
            if let StageNode::Act(a) = &mut node {
                supply(a);
            }
            let errs = validate(&plan(vec![node], vec!["hits"]))
                .expect_err("an unapplied narrowing must refuse");
            assert!(
                errs.iter()
                    .any(|e| e.reason == RefusalReason::FilterNotApplicable
                        && e.detail.contains(expected)),
                "supplying {label} must be DECLINED, naming what it declined; got: {errs:?}"
            );
        }
    }

    /// The other half of the rule above: **the seven that moved are ADMITTED on the act they moved
    /// to.**
    ///
    /// `[added — 2026-08-14]` Without this, the amendment that stopped `doc_type` from being a
    /// modifier reads as a refusal getting stricter, and a later reader could satisfy the test above
    /// by refusing the resource filter EVERYWHERE — including on the one act whose parameter it now
    /// is. That would pass the whole suite and delete the feature.
    ///
    /// A refusal that stops firing has to be shown to have stopped for the right reason, and the
    /// right reason here is that the narrowing has a home rather than that it was dropped.
    #[test]
    fn the_narrowings_a_find_act_refuses_are_admitted_on_find_resources_with() {
        let mut node = act("sel", ActName::FindResourcesWith, None);
        if let StageNode::Act(a) = &mut node {
            // Every field at once, including a MULTI-VALUE doc_type — the case the find fragments
            // called inexpressible, and the capability this act was built to add.
            a.resource_filter = Some(ResourceFilter {
                doc_type: vec!["task".to_string(), "session".to_string()],
                tags: vec!["ci".to_string()],
                facets: vec![FacetPredicate {
                    key: "domain".to_string(),
                    value: "search".to_string(),
                }],
                stage: Some("in-progress".to_string()),
                status: Some("active".to_string()),
                owner: Some("@someone".to_string()),
                title_contains: Some("door".to_string()),
            });
        }
        // Returned nowhere: a selection orders nothing, so asking for its rows is
        // `StageNotReturnable`. This plan is about what the FILTER does, so it returns no stage —
        // which `NoReturns` would refuse, so the selection is piped into a find act that IS
        // returnable, which is also the shape a caller would really write.
        let mut sink = act("hits", ActName::FindExact, None);
        if let StageNode::Act(a) = &mut sink {
            a.inputs = vec![StageInput::Upstream {
                relation: StageRelation::Bound,
                stage: StageName::parse("sel").unwrap(),
            }];
        }
        let result = validate(&plan(vec![node, sink], vec!["hits"]));
        assert!(
            result.is_ok(),
            "every resource narrowing is a parameter of `find-resources-with`, and a plan \
             selecting with one and piping it into a find act is legal; got: {result:?}"
        );
    }

    /// **The one filter field whose LENGTH is a cost multiplier is capped.**
    ///
    /// `[added — 2026-08-14]` Each facet predicate is evaluated against every candidate row, so cost
    /// is `|visible set| × |facets|` and an authenticated caller picks the second factor. Refused
    /// rather than clamped: clamping answers a different question silently, which is the whole thing
    /// this act exists to stop.
    ///
    /// The boundary is asserted on both sides, so a cap that drifted to "any facets at all" or to
    /// "no cap" both fail here rather than one of them passing quietly.
    #[test]
    fn a_selection_caps_the_facet_predicates_that_multiply_per_row_cost() {
        let facets = |n: usize| {
            (0..n)
                .map(|i| FacetPredicate {
                    key: format!("k{i}"),
                    value: "v".to_string(),
                })
                .collect::<Vec<_>>()
        };
        let plan_with = |n: usize| {
            let mut node = act("sel", ActName::FindResourcesWith, None);
            if let StageNode::Act(a) = &mut node {
                a.resource_filter = Some(ResourceFilter {
                    facets: facets(n),
                    ..Default::default()
                });
            }
            let mut sink = act("hits", ActName::FindExact, None);
            if let StageNode::Act(a) = &mut sink {
                a.inputs = vec![StageInput::Upstream {
                    relation: StageRelation::Bound,
                    stage: StageName::parse("sel").unwrap(),
                }];
            }
            plan(vec![node, sink], vec!["hits"])
        };

        assert!(
            validate(&plan_with(32)).is_ok(),
            "the cap itself must be admitted, or the boundary is off by one"
        );
        let errs = validate(&plan_with(33)).expect_err("one past the cap must refuse");
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::FilterNotApplicable
                    && e.detail.contains("facet predicates")),
            "the refusal must name what it declined and why; got: {errs:?}"
        );
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
        clear_intentions(&mut c);
        let errs = validate(&c).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.reason == RefusalReason::MissingIntention));

        set_intentions(&mut c, "salience");
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
        //
        // The act is incidental — the check reads the id SET — and moving it to a find act would
        // trade one companion refusal for another, since no act accepts a region bound either.
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
        // unbuildable. `cogmap in -> region out` is a legal SHAPE today with no SQL change.
        //
        // `[re-pointed — 2026-08-12]` This asserted `validate(&c).is_ok()`, and that stopped being
        // true when `survey` left `CALLABLE_FRAGMENTS`: the only act that produces regions is now
        // unreachable from this surface, so **no plan `validate` accepts changes kind at all**, and
        // there is no reachable act to move this example onto. That is a fact about the flip, and
        // this is where it is recorded rather than absorbed.
        //
        // The SUBJECT is expressibility and is unchanged — what changed is that the two questions
        // are now two functions. `validate_shape` is the published contract's answer and says
        // nothing about this hop; `validate`'s single refusal is this DOOR's capability and reads
        // `NotSeparablyReachable`, naming a fragment rather than anything about kinds. Foreclosure
        // would look like a SHAPE refusal, and there is none.
        let c = plan(
            vec![act(
                "shape",
                ActName::Survey,
                Some(caller_ids(IdKind::Cogmap)),
            )],
            vec!["shape"],
        );

        let shape = validate_shape(&c);
        assert!(
            shape.is_empty(),
            "a kind-changing hop must stay EXPRESSIBLE; the shape pass refused it: {shape:?}"
        );

        let errs = validate(&c).expect_err("survey has no fragment this surface can emit");
        assert_eq!(errs.len(), 1, "got: {errs:?}");
        assert_eq!(
            errs[0].reason,
            RefusalReason::NotSeparablyReachable,
            "the only thing between this hop and execution is a fragment this door has not \
             built; anything else would mean the shape itself was refused"
        );
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
    fn an_act_whose_fragment_takes_arguments_no_slot_supplies_refuses_statically() {
        // `follow-from` and `survey` mapped to a placeholder function that does not exist, so they
        // validated clean and failed at EXECUTION. Invisible while nothing executed a composition
        // outside its own tests; a 500 the moment a door opened.
        //
        // The flip is not primarily about the 500. `registry.rs` declares both acts
        // `DoorReach::Absent` at all three doors and promises they restore to `Serves` when this
        // door lands. Had the placeholder survived, BOTH would have been false: reachable through
        // the door, and unable to answer.
        // `[narrowed to survey — 2026-08-14]` `follow-from` was the other member and has left:
        // `query_follow_from` is in `CALLABLE_FRAGMENTS` and `registry.rs` now declares `Serves` at
        // CLI and API, which is the restoration the comment above promised. The pairing is what
        // made the promise checkable, so the assertion follows the act out rather than being
        // loosened to keep it passing.
        let c = a_legal_single_stage_plan_over(ActName::Survey);
        let errs = validate(&c).expect_err("the act has no fragment this surface can emit");
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::NotSeparablyReachable),
            "survey must refuse as unreachable rather than compile to an absent function; \
             got {errs:?}"
        );
    }

    /// The other half of the flip above: **`follow-from` no longer refuses**, and the reason it
    /// stopped is the one the map records rather than a general loosening `[2026-08-14]`.
    ///
    /// Without this, the test above could be made to pass by deleting an act from a list, and
    /// nothing would notice that the act had become reachable for a bad reason — or not at all.
    #[test]
    fn follow_from_is_no_longer_refused_as_unreachable() {
        let c = a_legal_single_stage_plan_over(ActName::FollowFrom);
        if let Err(errs) = validate(&c) {
            assert!(
                !errs
                    .iter()
                    .any(|e| e.reason == RefusalReason::NotSeparablyReachable),
                "`query_follow_from` is in CALLABLE_FRAGMENTS, so this surface can emit it; \
                 got {errs:?}"
            );
        }
    }

    // ---- The relation moved onto the edge ---------------------------------------------------

    #[test]
    fn asking_an_act_to_reach_beyond_a_set_it_can_only_narrow_within_is_refused() {
        // The negative face of carrying the relation on the wire at all.
        //
        // **The argument for it used to be principle and is now also FORCE** `[2026-08-14]`. This
        // read: "across the seven acts `accepts_bounds` and `accepts_seeds` are DISJOINT, so the
        // relation is fully determined by the act and could have been derived — and deriving it is
        // precisely the mistake." They are no longer disjoint: `follow-from` accepts a `Resource`
        // seed AND a `Resource` bound, and carries both at once. So a derived relation is not
        // merely wrong in principle, it is now unable to answer — which is worth recording, because
        // a design decision taken on principle and later vindicated by necessity is one nobody
        // should have to re-argue. A
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
        set_intentions(&mut c, "composable fragments");
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
            up.inputs[0].relation(),
            StageRelation::Seed,
            "an upstream edge carries its own relation"
        );
        assert_eq!(
            ca.inputs[0].relation(),
            StageRelation::Bound,
            "so does a caller-supplied one"
        );

        // **The negative half needed a new subject** `[2026-08-14]`. It used to be this same
        // `follow-from` plan: the act accepted seeds and not bounds, so a `Bound` relation refused
        // as `UnsupportedBoundKind`. It now accepts BOTH — a resource seed to grow from and a
        // resource bound to stay within — so that plan is legal and the assertion would have had to
        // be deleted to go green.
        //
        // Re-pointed instead, at an act that genuinely does not take a resource bound: `survey`
        // accepts `[Cogmap, Context]`, which are served by the anchor pair. The rule under test is
        // the relation being READ, and it needs a case where reading it changes the answer.
        let survey_resource_bound = act_with_relation(
            "regions",
            ActName::Survey,
            Some(caller_ids(IdKind::Resource)),
            StageRelation::Bound,
        );
        assert!(
            validate(&plan(vec![survey_resource_bound], vec!["regions"]))
                .unwrap_err()
                .iter()
                .any(|e| e.reason == RefusalReason::UnsupportedBoundKind),
            "survey scopes to a cogmap or a context, never to a set of resources"
        );

        // And the plan that used to be refused is now WELL-FORMED as far as the relation goes —
        // asserted as the absence of that refusal, so this reads as a capability that opened rather
        // than as a check that was dropped.
        if let Err(errs) = validate(&plan(vec![caller_bound], vec!["hits"])) {
            assert!(
                !errs
                    .iter()
                    .any(|e| e.reason == RefusalReason::UnsupportedBoundKind),
                "follow-from now accepts a resource bound — it constrains the whole walk; \
                 got {errs:?}"
            );
        }
    }

    // ---- Hydration sections -----------------------------------------------------------------

    #[test]
    fn a_section_this_door_does_not_hydrate_is_refused_with_somewhere_to_go() {
        // `body` is a real word in the shared vocabulary, so it deserializes — and is refused
        // here, which is the point of not making it a narrow query-local enum. A refusal that only
        // said no would leave the caller guessing; this one names `show`.
        let mut c = plan_returning(
            vec![act(
                "hits",
                ActName::FindExact,
                Some(caller_ids(IdKind::Resource)),
            )],
            vec![ReturnSpec {
                stage: StageName::parse("hits").unwrap(),
                with: vec![ResourceSection::Body],
            }],
        );
        set_intentions(&mut c, "q");
        let errs = validate(&c).unwrap_err();
        let hit = errs
            .iter()
            .find(|e| e.reason == RefusalReason::SectionNotAvailable)
            .unwrap_or_else(|| panic!("got: {errs:?}"));
        assert_eq!(hit.stage.as_ref().unwrap().as_str(), "hits");
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
        let mut c = plan_returning(
            vec![act("hits", ActName::FindExact, upstream("ghost"))],
            vec![ReturnSpec {
                stage: StageName::parse("hits").unwrap(),
                with: vec![ResourceSection::Body, ResourceSection::Edges],
            }],
        );
        set_intentions(&mut c, "q");
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
        set_intentions(&mut c, "salience");
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
        set_intentions(&mut c, "composable fragments");
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
        //
        // `[re-pointed — 2026-08-12]` The subject was `survey`, and the promise was read through
        // `ValidationOutcome`. The flip made that impossible rather than merely awkward: the
        // reachable set is now exactly the three find acts, every one of which produces Resources
        // on a UNIT-interval score, so **no plan `validate` accepts can carry a Regions variant or
        // a non-unit range through the outcome at all**. Split rather than weakened — the
        // derivation keeps its witness over a reachable act, and the region case is asserted one
        // layer down, against the same two accessors `ValidationOutcome::of` reads.
        let c = plan_with_intention(vec![act("hits", ActName::FindExact, None)], vec!["hits"]);
        let out = ValidationOutcome::of(&validate(&c).expect("legal"));
        let hits = &out.will_return[&StageName::parse("hits").unwrap()];

        assert_eq!(hits.produced, ProducedVariant::Resources);
        assert_eq!(
            hits.score_kind.as_ref().map(ScoreKind::as_str),
            hits.orders_by.as_ref().map(|q| q.field.as_str())
        );
        // And the range is carried rather than left to a convention about the name.
        assert!(matches!(
            hits.orders_by.as_ref().map(|q| &q.scale),
            Some(crate::types::query::QuantityScale::UnitInterval)
        ));

        // The region case, at the layer that survives the flip. `region_score` is NOT [0,1] and its
        // name does not say so, which is exactly why the pairing has to hold for THIS act — and
        // `survey` is the only act it can be asked about.
        let survey = declaration(&ActName::Survey).expect("survey is declared");
        assert_eq!(
            survey.score_kind().as_ref().map(ScoreKind::as_str),
            survey.orders_by.as_ref().map(|q| q.field.as_str())
        );
        assert!(matches!(
            survey.orders_by.as_ref().map(|q| &q.scale),
            Some(crate::types::query::QuantityScale::OtherRange { .. })
        ));
        // **And the variant, which is the third fact and not a spare one.** The region half that
        // travelled through `ValidationOutcome` asserted `produced == Regions`, and that was the
        // only assertion in the workspace pinning `produced_variant`'s `IdKind::Region` arm:
        // `every_selecting_act_predicts_a_response_shape` asserts `is_some()`, and
        // `the_promised_variant_is_the_one_a_real_output_reports` reads `StageOutput::variant()`
        // rather than a declaration. Without this line, mutating that arm to `Resources` leaves the
        // whole workspace green — the wrong promise `act.rs`'s own comment says is worse than an
        // absent one, and a promise no reachable plan can be handed is exactly where nobody looks.
        assert_eq!(survey.produced_variant(), Some(ProducedVariant::Regions));
    }

    #[test]
    fn the_outcome_carries_exactly_the_keys_returns_asked_for_no_more_no_fewer() {
        // A stage that only FEEDS a downstream one is never hydrated and must not appear here —
        // promising a key the response will not carry is the same class of lie as omitting one it
        // will.
        let mut c = plan(
            vec![
                act("seeds", ActName::FindAboutAnywhere, None),
                act("narrowed", ActName::FindAboutWithin, upstream("seeds")),
            ],
            vec!["narrowed"],
        );
        set_intentions(&mut c, "x");
        let out = ValidationOutcome::of(&validate(&c).expect("legal"));

        assert_eq!(out.will_return.len(), 1);
        assert!(out
            .will_return
            .contains_key(&StageName::parse("narrowed").unwrap()));
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
                StageName::parse("narrowed").unwrap()
            ]
        );
    }

    #[test]
    fn the_outcome_tells_a_caller_which_optional_fields_will_be_null() {
        // The reason `discloses` exists at all: a caller learns which optional fields will be
        // filled HERE rather than by finding a null in a response and guessing whether it means
        // "cannot" or "none".
        //
        // `find-exact` declares nothing, and the reason is not that it has nothing to say: a find
        // act is exactly the kind that COULD report where in a resource it matched, and
        // `registry.rs`'s `match_location_is_declared_by_no_act_until_the_wide_fragments_carry_the_
        // chunk_out` holds every act to declaring nothing until the executor fills it. So this is a
        // promise of an empty set, made deliberately.
        let c = plan_with_intention(
            vec![act(
                "hits",
                ActName::FindExact,
                Some(caller_ids(IdKind::Resource)),
            )],
            vec!["hits"],
        );
        let out = ValidationOutcome::of(&validate(&c).expect("legal"));
        let near = &out.will_return[&StageName::parse("hits").unwrap()];

        assert_eq!(near.produced, ProducedVariant::Resources);
        assert_eq!(near.score_kind, Some(ScoreKind::FtsNorm));
        assert!(
            near.discloses.is_empty(),
            "find-exact declares no disclosure; got: {:?}",
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
        let c = plan_with_intention(
            vec![
                act("a", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
                act("b", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
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
            errs.iter()
                .any(|e| e.reason == RefusalReason::StageNotReturnable
                    && e.stage.as_ref().is_some_and(|s| s.as_str() == "merged")),
            "got: {errs:?}"
        );
    }

    #[test]
    fn a_combinator_that_is_not_returned_stays_legal() {
        // The boundary: combining is the point of having combinators. Only asking for the combined
        // rows BACK is refused, and a composition that unions two stages and returns one of them is
        // exactly the shape the DAG exists for.
        let c = plan_with_intention(
            vec![
                act("a", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
                act("b", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
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

    // `the_two_placeholder_fused_acts_are_not_refused_by_reading_their_host` stood here and is
    // DELETED rather than re-pointed, because the flip did not move its subject — it falsified it.
    // The test asserted that `survey` and `follow-from` validate clean, on the argument that the two
    // `Fused` declarations were among the reachable ones and so a rule keyed on that discriminant
    // would refuse the wrong acts. Both acts are now refused, deliberately, and
    // `an_act_whose_fragment_takes_arguments_no_slot_supplies_refuses_statically` asserts exactly
    // that over exactly them.
    //
    // What the deleted test also carried — that reachability is keyed on served-by names and never
    // on `build_state` — is NOT retired with it, but it now has one witness rather than two:
    // `a_served_act_this_builder_has_no_fragment_for_refuses_honestly`, where `substantiate` is
    // `Served` and unreachable. There is no `Fused`-and-reachable act left to witness the other
    // direction, and CALLABLE_FRAGMENTS' doc comment says so rather than implying both.

    // ---- The shape/capability seam ----------------------------------------------------------

    #[test]
    fn the_shape_pass_answers_without_consulting_any_declaration() {
        // A plan whose ONLY problem is that its act is not reachable from this surface. The
        // capability pass refuses it; the shape pass must not, because a client one release behind
        // would then decline a plan the server would run.
        //
        // `substantiate` is the subject rather than `survey`: it is `Served` by
        // `resource_standing_shape`, which is absent from `CALLABLE_FRAGMENTS`, and its
        // `accepts_*` lists are all empty — so a minimal plan over it raises exactly one refusal,
        // and that refusal is `NotSeparablyReachable`.
        let c = a_legal_single_stage_plan_over(ActName::Substantiate);

        let shape = validate_shape(&c);
        assert!(
            shape.is_empty(),
            "the shape pass raised {shape:?} for a well-formed plan; only expressibility belongs here"
        );

        let full = validate(&c).expect_err("the capability pass refuses an unreachable mechanic");
        assert!(
            full.iter()
                .any(|e| e.reason == RefusalReason::NotSeparablyReachable),
            "expected the capability pass to supply the refusal the shape pass withheld; got {full:?}"
        );
    }

    #[test]
    fn an_unknown_act_name_is_refused_from_the_type_rather_than_from_the_registry() {
        // `ActName` is open (`act.rs:45-46`), so `"act": "made-up"` deserializes into `Other`
        // instead of failing serde — this is a caller-reachable refusal, and it is answerable
        // without the registry, which is what lets it sit in the shape pass.
        //
        // Pinned because the check was REWRITTEN when the passes split: it used to be
        // `declaration(&inv.act)` returning `None`, and it is now `matches!(inv.act,
        // ActName::Other(_))`. The two agree only because `search_family()` declares every
        // non-`Other` variant, so the equivalence is load-bearing and had no witness at all.
        let c = a_legal_single_stage_plan_over(ActName::Other("made-up".to_string()));

        let shape = validate_shape(&c);
        assert_eq!(shape.len(), 1, "got: {shape:?}");
        assert_eq!(shape[0].reason, RefusalReason::UnknownAct);
        assert_eq!(shape[0].stage.as_ref().map(|s| s.as_str()), Some("s"));

        // And the capability pass adds nothing: an act with no declaration is nothing it can
        // speak about, so the caller gets the one refusal that means something.
        let errs = validate(&c).expect_err("an act nobody declares is not a plan");
        assert_eq!(errs, shape, "got: {errs:?}");
    }

    #[test]
    fn a_cyclic_plan_still_hears_about_a_section_this_door_does_not_hydrate() {
        // The `returns` check reads no stage graph — it compares `with` against a constant — so
        // a cycle takes nothing away from its answer, and swallowing it would hand this caller
        // one of their two problems. `validate` returns EVERY refusal.
        //
        // Beside `a_refused_section_comes_back_alongside_every_other_refusal_not_instead_of_them`
        // rather than replacing it: that one's subject is a dangling reference, whose graph still
        // topologically sorts, so it cannot see this gate at all.
        let c = plan_returning(
            vec![
                act("a", ActName::FindExact, upstream("b")),
                act("b", ActName::FindExact, upstream("a")),
            ],
            vec![ReturnSpec {
                stage: StageName::parse("a").unwrap(),
                with: vec![ResourceSection::Body],
            }],
        );
        let errs = validate(&c).unwrap_err();
        assert!(
            errs.iter().any(|e| e.detail.contains("cycle")),
            "got: {errs:?}"
        );
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::SectionNotAvailable),
            "a cycle must not swallow a refusal that never asked about the graph; got: {errs:?}"
        );
    }

    #[test]
    fn the_shape_pass_still_refuses_a_malformed_plan() {
        // The other direction: a shape refusal must not have migrated into the capability pass,
        // where `--check` would never see it.
        let mut c = a_legal_single_stage_plan_over(ActName::Substantiate);
        c.outcome.returns.clear();

        let shape = validate_shape(&c);
        assert!(
            shape
                .iter()
                .any(|e| e.reason == RefusalReason::NoReturns),
            "a composition that answers nothing is malformed regardless of what is built; got {shape:?}"
        );
    }
}
