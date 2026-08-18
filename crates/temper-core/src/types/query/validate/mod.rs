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
/// `[2026-08-16]` `follow-from` and `survey` both LEFT this map's absent set — `follow-from` on
/// 2026-08-14, `survey` on 2026-08-16 — so the only act absent from it is `substantiate`, which is
/// `Served` and unreachable. The `Fused` discriminant no longer agrees with this map about anything,
/// because nothing absent from this map carries it. One direction, said as one direction.
///
/// `[2026-08-16]` **`survey` JOINED this map.** It was absent through beat C because
/// `wayfind_region_scores` takes a `p_lens` no slot supplies. The `p_lens` blocker was settled:
/// `NULL` is correct, not just a default — the lens is a clustering-time parameter, and `NULL` at
/// query time reads the baked salience (verified from the SQL body and production data). The
/// `query_survey` wrapper (migration 20260816000020) passes `p_lens = NULL` as a constant and
/// joins matched regions to member resources. Survey now produces RESOURCES (the ratified ⟨3⟩
/// redesign), not regions; regions move to `discloses`. See the design spec (01a00c0b-200c).
///
/// `[joined — 2026-08-14]` **`follow-from` left the absent set.** It was absent for the same
/// reason — `search_graph_expand` takes `p_depth`/`p_gamma` no slot supplies — and the answer was
/// not a slot for either. Both are DEFINITIONAL rather than caller inputs (the act fixes depth at 2
/// and gamma at the rate its `orders_by` sentence describes), so `query_follow_from` takes them and
/// the compiler passes constants. What the act needed a slot for was the seed set it already had,
/// and the bound it did not — `20260814000030` plus `inputs: Vec<StageInput>`.
///
/// `[added — 2026-08-14]` `find-resources-with` joins as the third member. It is the first entry
/// whose fragment takes NO intention and returns NO quantity — the map says nothing about either,
/// which is why it needed no widening to admit one.
///
/// `[added — 2026-08-16]` `survey` joins as the fifth member. It is the first entry whose fragment
/// takes BOTH `p_visible_ids` and `p_principal` — the others take only `p_visible_ids` — because
/// `wayfind_region_scores` applies its own region visibility by principal. The `p_principal` is
/// the compiler's `$1`, not a second id set.
const CALLABLE_FRAGMENTS: &[(&str, &str)] = &[
    ("query_find_exact", "__temper_ungated_find_exact"),
    (
        "query_find_resources_with",
        "__temper_ungated_find_resources_with",
    ),
    ("query_find_wide", "__temper_ungated_find_wide"),
    ("query_follow_from", "__temper_ungated_follow_from"),
    ("query_survey", "__temper_ungated_survey"),
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
        EdgeFilter, FacetPredicate, PropertyOp, PropertyPredicate, ResourceFilter,
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
        // This asserts the ABSENCE of the duplicate refusal rather than success, so that it holds
        // whether or not `follow-from` is refused for some other reason. Naming the reason is what
        // makes it survive those other refusals retiring — and two of them since have.
        //
        // `[corrected — 2026-08-15]` The list of "other reasons" here read *"it is absent from
        // `CALLABLE_FRAGMENTS` and declares `accepts_bounds: []`"*, and **both halves stopped being
        // true on 2026-08-14**: `query_follow_from` entered `CALLABLE_FRAGMENTS`, and the act
        // declares `accepts_bounds: vec![IdKind::Resource]` — the bounded walk `20260814000030`
        // built. The `if let Err` below is what let the prose rot silently: the test passes
        // identically whether the plan validates or not, so nothing ever re-read the reasons it
        // claimed. The shape is left as it stands — the assertion is about one refusal's absence,
        // not about the plan's overall verdict — with the stale justification repaired rather than
        // deleted, since it is the part a reader trusts.
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
            ActName::FindExact
                | ActName::FindAboutAnywhere
                | ActName::FindAboutWithin
                | ActName::Survey
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

    /// A predicate on the TOMBSTONE — `ActInvocation::properties`, which every act refuses.
    fn plan_with_property(key: &str, op: PropertyOp) -> Composition {
        let mut node = act("s", ActName::FindExact, Some(caller_ids(IdKind::Resource)));
        if let StageNode::Act(a) = &mut node {
            a.properties.push(PropertyPredicate {
                key: key.to_string(),
                op,
            });
        }
        plan_with_intention(vec![node], vec!["s"])
    }

    /// A resource filter on the one act that applies it, piped into a returnable find act.
    ///
    /// The sink is not decoration: a selection orders nothing, so returning it directly is
    /// `StageNotReturnable` and every assertion below would pass for the wrong reason.
    fn selection_plan(f: ResourceFilter) -> Composition {
        let mut node = act("sel", ActName::FindResourcesWith, None);
        if let StageNode::Act(a) = &mut node {
            a.resource_filter = Some(f);
        }
        let mut sink = act("hits", ActName::FindExact, None);
        if let StageNode::Act(a) = &mut sink {
            a.inputs = vec![StageInput::Upstream {
                relation: StageRelation::Bound,
                stage: StageName::parse("sel").unwrap(),
            }];
        }
        plan(vec![node, sink], vec!["hits"])
    }

    /// A predicate in the RESOURCE CONTAINER, on the one act that applies it.
    fn selection_with_properties(props: Vec<PropertyPredicate>) -> Composition {
        selection_plan(ResourceFilter {
            properties: props,
            ..Default::default()
        })
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
        // `[retired — 2026-08-16]` This test piped survey→find-exact to assert
        // `UnsupportedBoundKind`, because survey produced `Region` and find-exact does not accept
        // `Region` bounds. Survey now produces `Resource` (the ratified ⟨3⟩ redesign), and
        // find-exact accepts `Resource` — so the kind mismatch is gone. No act in the registry
        // today produces a kind another act does not accept, so the `UnsupportedBoundKind` refusal
        // is unexercisable by a kind-changing hop. The refusal path still exists (the validate
        // layer checks it); it just has no live subject. Kept as a retirement note rather than
        // deleted, because the category-error refusal is a property that should be witnessed again
        // if a future act reintroduces a kind mismatch.
        //
        // What this test DOES now: confirms survey→find-exact is a VALID pipe (resource→resource),
        // which is the natural agent flow the redesign enabled. Both stages need intentions; the
        // plan refuses on `MissingIntention`, not on a kind mismatch.
        let c = plan(
            vec![
                act("shape", ActName::Survey, Some(caller_ids(IdKind::Cogmap))),
                act("hits", ActName::FindExact, upstream("shape")),
            ],
            vec!["hits"],
        );
        // Survey now produces resources, which find-exact accepts — so this plan validates clean.
        // The `UnsupportedBoundKind` refusal is unexercisable here because there is no kind
        // mismatch. This assertion confirms that: the plan is accepted, not refused.
        validate(&c).expect("survey->find-exact is a valid resource->resource pipe now");
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
        // `[amended — 2026-08-16]` This used to also earn `NotSeparablyReachable` because survey
        // was absent from `CALLABLE_FRAGMENTS`. Survey is now reachable, so the only refusals are
        // `BoundTermNotApplicable` (limit) and `MissingIntention` (no query on the stage). The
        // assertion checks the limit refusal is among them.
        let mut node = act("shape", ActName::Survey, Some(caller_ids(IdKind::Cogmap)));
        if let StageNode::Act(a) = &mut node {
            a.terms.insert(BoundTerm::Limit, 10);
        }
        let errs = validate(&plan(vec![node], vec!["shape"])).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.reason == RefusalReason::BoundTermNotApplicable));
    }

    /// **A bound term an act does not admit is refused before execution.** The admission arm of
    /// `BoundTermNotApplicable` — `!decl.accepts_bound_terms.contains(term)` in `capability.rs` —
    /// had no witness anywhere in the tree. The negative-value arm was witnessed
    /// ([`a_negative_paging_term_is_refused_rather_than_handed_to_postgres`]) and the declaration
    /// arm was witnessed by [`survey_declines_limit_because_its_bound_is_a_funnel_width`], but
    /// nothing would go red if `survey` or `find-resources-with` silently gained `Offset`.
    ///
    /// `[added — 2026-08-18]` PR #708 ADDED a term to an act, which makes the blast radius of
    /// "which acts admit which terms" exactly the thing that is unguarded. `find-resources-with`
    /// declares `accepts_bound_terms: vec![]` while being row-returning — the shape most likely to
    /// acquire one by mistake — and `survey` admits `Regions` and not `Offset`. Both must refuse
    /// `Offset` statically, before execution.
    ///
    /// Parameterised over both acts so the test fails if the admission check is removed for
    /// EITHER an act that admits no terms at all or one that admits a different term.
    #[test]
    fn an_offset_on_an_act_that_does_not_admit_it_is_refused_before_execution() {
        for a in [ActName::FindResourcesWith, ActName::Survey] {
            let mut node = act("s", a.clone(), None);
            if let StageNode::Act(inv) = &mut node {
                inv.terms.insert(BoundTerm::Offset, 10);
            }
            let c = plan_with_intention(vec![node], vec!["s"]);
            let errs = validate(&c).unwrap_err();
            assert!(
                errs.iter()
                    .any(|e| e.reason == RefusalReason::BoundTermNotApplicable),
                "{a:?} does not admit `offset` and must refuse it statically, before execution; \
                 got: {errs:?}"
            );
        }
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
    /// a case may legitimately raise `UnknownFilterValue` beside the refusal under test.
    /// `[stale — 2026-08-15]` That variant is DELETED and nothing raises it; the "closed
    /// vocabularies" half was already false per ADJ-10. The reason this helper still filters by
    /// discriminant is unchanged — a plan can raise more than one refusal at once — but the example
    /// it gives no longer exists. The
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
                        key: "k".to_string(),
                        op: PropertyOp::HasKey,
                    }]
                }),
                "belongs in a filter container",
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
                        properties: vec![],
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
                // The open-key slot, in the same "every field at once" case: it is a narrowing this
                // act admits, not a special one bolted beside them.
                properties: vec![PropertyPredicate {
                    key: "derived_from".to_string(),
                    op: PropertyOp::Contains {
                        values: vec![serde_json::json!("spec-a")],
                    },
                }],
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
                    && e.detail.contains("per-row predicates")
                    && e.detail.contains("33 facet")),
            "the refusal must name what it declined and why — and since 2026-08-15 it must break \
             the count down by field, because the cap now counts two of them together; got: {errs:?}"
        );
    }

    #[test]
    fn an_edge_filter_on_a_resource_only_act_is_declined_not_ignored() {
        let mut node = act("hits", ActName::FindExact, None);
        if let StageNode::Act(a) = &mut node {
            a.edge_filter = Some(EdgeFilter {
                properties: vec![],
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
        // `[re-pointed again — 2026-08-16]` Survey is now REACHABLE and produces RESOURCES, not
        // regions (the ratified ⟨3⟩ redesign). So `cogmap in -> resource out` is both expressible
        // AND capability-reachable: `validate` accepts it. The "kind-changing hop" framing no
        // longer applies — survey takes a cogmap bound and produces resources, which is a
        // kind-changing hop (cogmap→resource) that is now fully reachable. The test asserts both
        // passes are clean.
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

        // `[re-pointed again — 2026-08-16]` Survey is now REACHABLE and produces RESOURCES, not
        // regions (the ratified ⟨3⟩ redesign). `act()` now supplies a natural intention for survey
        // (via `natural_intention`), so `validate` accepts this plan — it is a valid cogmap→resource
        // hop. The "kind-changing hop" framing no longer applies as a refusal; it is now a valid
        // composition.
        validate(&c).expect("survey with an intention validates clean — it is reachable now");
    }

    // ---- Task 7b: the property predicate ----------------------------------------------------

    #[test]
    fn a_stale_subject_tagged_predicate_still_reaches_a_named_refusal() {
        // **The tombstone, end to end.** `PropertySubject` is deleted, so `content_block` — which
        // spec §12 refused as `UnknownFilterValue` because a block is addressable and not
        // queryable — no longer has an enum arm to be unknown in. What must NOT happen is that the
        // stale body becomes unparseable: `ActInvocation` carries `deny_unknown_fields`, so if the
        // field itself had been removed this plan would die in serde with a 400 outside
        // `ErrorBody`, and the caller would be told their request is malformed rather than where
        // their predicate now belongs.
        let wire = serde_json::json!({
            "stages": [{
                "name": "s",
                "act": "find-exact",
                "intention": { "query": "anything" },
                "properties": [{
                    "subject": "content_block",
                    "key": "block_role",
                    "op": { "op": "has_key" }
                }]
            }],
            "outcome": { "returns": [{ "stage": "s" }] }
        });
        let c: Composition =
            serde_json::from_value(wire).expect("a stale subject tag must still parse");
        let errs = validate(&c).unwrap_err();
        let redirect = errs
            .iter()
            .find(|e| e.reason == RefusalReason::FilterNotApplicable)
            .expect("the tombstone must refuse, and by name");
        // The refusal has to say where the capability went. It can no longer say WHICH container —
        // the tag that would have told it apart is gone — so it must name both.
        assert!(
            redirect.detail.contains("resource_filter.properties")
                && redirect.detail.contains("edge_filter.properties"),
            "a refusal that does not say where the capability went is a dead end; got {}",
            redirect.detail
        );
        // The refusal it replaced (`UnknownFilterValue`) is gone from the enum entirely, not merely
        // unreachable — deleting the variant is what makes that a compile-time fact rather than a
        // thing this assertion would have to keep checking.
    }

    #[test]
    fn an_empty_property_key_is_refused_rather_than_matching_everything() {
        let c = plan_with_property("", PropertyOp::HasKey);
        assert!(validate(&c).is_err());
    }

    #[test]
    fn contains_with_no_values_is_refused_because_it_narrows_nothing() {
        // An empty list is not "match all" and is not "match none" — it is a caller mistake, and
        // silently treating it as either is the confident-empty failure this contract exists to end.
        let c = plan_with_property("tags", PropertyOp::Contains { values: vec![] });
        assert!(validate(&c).is_err());
    }

    #[test]
    fn a_malformed_predicate_inside_a_container_is_refused_too() {
        // **The gap this half surfaced** `[fixed — 2026-08-15]`. The shape pass read only
        // `inv.properties`, written when that was the only place a predicate could sit. So a
        // container predicate with an empty key or an empty `contains` passed shape validation
        // entirely and COMPILED — to `property_key = ''`, or to an `EXISTS` over an empty array.
        // Both narrow to nothing silently, which is exactly what these two refusals exist to
        // prevent.
        //
        // It matters more now than when it shipped: with both containers built and the invocation
        // field refused outright, checking only the tombstone would leave `EmptyPropertyKey` and
        // `EmptyContains` firing ONLY where no predicate can apply.
        for (label, op, key) in [
            ("an empty key", PropertyOp::HasKey, ""),
            (
                "an empty contains",
                PropertyOp::Contains { values: vec![] },
                "derived_from",
            ),
        ] {
            let c = selection_with_properties(vec![PropertyPredicate {
                key: key.to_string(),
                op,
            }]);
            assert!(
                validate(&c).is_err(),
                "{label} inside resource_filter.properties was accepted"
            );
        }

        // The edge container, which is where the gap actually shipped.
        let mut node = act("w", ActName::FollowFrom, None);
        if let StageNode::Act(a) = &mut node {
            a.inputs = vec![StageInput::Caller {
                relation: StageRelation::Seed,
                ids: IdSet {
                    kind: IdKind::Resource,
                    provenance: None,
                    ids: vec![Uuid::now_v7()],
                },
            }];
            a.edge_filter = Some(EdgeFilter {
                properties: vec![PropertyPredicate {
                    key: String::new(),
                    op: PropertyOp::HasKey,
                }],
                ..Default::default()
            });
        }
        let c = plan_with_intention(vec![node], vec!["w"]);
        assert!(
            validate(&c).is_err(),
            "an empty key inside edge_filter.properties was accepted"
        );

        // Positive control: the same containers with a well-formed predicate validate, so the
        // errors above are the shape checks and not the plans being broken some other way.
        assert!(validate(&selection_with_properties(vec![PropertyPredicate {
            key: "derived_from".to_string(),
            op: PropertyOp::Contains {
                values: vec![serde_json::json!("spec-a")],
            },
        }]))
        .is_ok());
    }

    #[test]
    fn the_per_candidate_cap_counts_facets_and_open_key_predicates_together() {
        // `[2026-08-15]` The cap bounds the SECOND FACTOR of `|candidates| x |predicates|`, and
        // both of these walk the same candidate rows — so their costs ADD. Capping each at 32
        // separately would double the ceiling by omission rather than by anyone.
        let facets = |n: usize| -> Vec<FacetPredicate> {
            (0..n)
                .map(|i| FacetPredicate {
                    key: format!("k{i}"),
                    value: "v".to_string(),
                })
                .collect()
        };
        let props = |n: usize| -> Vec<PropertyPredicate> {
            (0..n)
                .map(|i| PropertyPredicate {
                    key: format!("p{i}"),
                    op: PropertyOp::HasKey,
                })
                .collect()
        };

        let errs = validate(&selection_plan(ResourceFilter {
            facets: facets(20),
            properties: props(20),
            ..Default::default()
        }))
        .expect_err("40 = 20 + 20 exceeds the cap");
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::FilterNotApplicable
                    && e.detail.contains("per-row predicates")),
            "the sum must cross the cap even though neither field does alone; got {errs:?}"
        );

        // Neither alone crosses it, which is what makes the assertion above about the SUM rather
        // than about either list being long.
        for (label, f) in [
            (
                "facets alone",
                ResourceFilter {
                    facets: facets(20),
                    ..Default::default()
                },
            ),
            (
                "open-key alone",
                ResourceFilter {
                    properties: props(20),
                    ..Default::default()
                },
            ),
        ] {
            assert!(
                validate(&selection_plan(f)).is_ok(),
                "{label} is under the cap and must be admitted"
            );
        }
    }

    #[test]
    fn a_long_value_list_is_capped_even_when_the_predicate_count_is_tiny() {
        // **The hole the predicate cap left, closed** `[2026-08-15, found in review]`. ONE predicate
        // is far under `MAX_PER_CANDIDATE_PREDICATES`, and its value list is what actually
        // multiplies the read: the fragment's inner `EXISTS` short-circuits only on a MATCH, so a
        // list that all misses is scanned in full for every row carrying the key.
        //
        // Measured on prod: one predicate with 2,000 non-matching values against `doc_type` (3,761
        // live rows) ran 1,628 ms and discarded 2,507,333 join-filter rows, against ~33 ms for the
        // same predicate carrying one value. The old cap admitted 32 of those.
        let errs = validate(&selection_plan(ResourceFilter {
            properties: vec![PropertyPredicate {
                key: "derived_from".to_string(),
                op: PropertyOp::Contains {
                    values: (0..300)
                        .map(|i| serde_json::json!(format!("miss-{i}")))
                        .collect(),
                },
            }],
            ..Default::default()
        }))
        .expect_err("300 probes exceeds the probe cap even though 1 predicate does not");
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::FilterNotApplicable
                    && e.detail.contains("probes per candidate")),
            "the probe cap must fire on the VALUE count, not the predicate count; got {errs:?}"
        );

        // Under the probe cap, the same single predicate is admitted — so the refusal above is the
        // list length and not the predicate merely existing.
        assert!(validate(&selection_plan(ResourceFilter {
            properties: vec![PropertyPredicate {
                key: "derived_from".to_string(),
                op: PropertyOp::Contains {
                    values: (0..20)
                        .map(|i| serde_json::json!(format!("v-{i}")))
                        .collect(),
                },
            }],
            ..Default::default()
        }))
        .is_ok());

        // `has_key` counts as ONE probe rather than zero — otherwise a caller pads the predicate
        // list for free against the other cap.
        assert!(validate(&selection_plan(ResourceFilter {
            properties: (0..10)
                .map(|i| PropertyPredicate {
                    key: format!("k{i}"),
                    op: PropertyOp::HasKey,
                })
                .collect(),
            ..Default::default()
        }))
        .is_ok());
    }

    #[test]
    fn a_malformed_predicate_refusal_names_which_container_it_came_from() {
        // `[2026-08-15, found in review]` With one source a bare "a property predicate needs a key"
        // identified the field by elimination. With three it does not — and a caller with a
        // malformed predicate in TWO containers on one stage would receive two byte-identical
        // refusals and no way to tell which to fix.
        let mut node = act("w", ActName::FollowFrom, None);
        if let StageNode::Act(a) = &mut node {
            a.inputs = vec![StageInput::Caller {
                relation: StageRelation::Seed,
                ids: IdSet {
                    kind: IdKind::Resource,
                    provenance: None,
                    ids: vec![Uuid::now_v7()],
                },
            }];
            a.edge_filter = Some(EdgeFilter {
                properties: vec![PropertyPredicate {
                    key: String::new(),
                    op: PropertyOp::HasKey,
                }],
                ..Default::default()
            });
            a.properties = vec![PropertyPredicate {
                key: String::new(),
                op: PropertyOp::HasKey,
            }];
        }
        let errs = validate(&plan_with_intention(vec![node], vec!["w"])).expect_err("both refuse");
        let details: Vec<&str> = errs
            .iter()
            .filter(|e| e.reason == RefusalReason::EmptyPropertyKey)
            .map(|e| e.detail.as_str())
            .collect();
        assert_eq!(details.len(), 2, "one per source; got {details:?}");
        // The tombstone's is `` `properties[0]` `` and the container's is
        // `` `edge_filter.properties[0]` ``. Matched on the backtick-anchored prefix, because the
        // container's spelling CONTAINS the tombstone's — a naive `contains` passes for both and
        // would not notice the two collapsing back into one.
        assert!(
            details
                .iter()
                .any(|d| d.contains("`edge_filter.properties[0]`"))
                && details
                    .iter()
                    .any(|d| d.contains("`properties[0]`") && !d.contains("edge_filter")),
            "each refusal must name its own source; got {details:?}"
        );
        // And they must not be byte-identical, which is the whole finding.
        assert_ne!(details[0], details[1]);
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
    fn survey_no_longer_refuses_statically() {
        // `[2026-08-16]` This was `an_act_whose_fragment_takes_arguments_no_slot_supplies_refuses_statically`,
        // and it asserted survey refused as `NotSeparablyReachable` because `wayfind_region_scores`
        // takes a `p_lens` no slot supplies. The `p_lens` blocker is settled: `NULL` is correct, not
        // just a default — the lens is a clustering-time parameter, and `NULL` at query time reads
        // the baked salience. The `query_survey` wrapper (migration 20260816000020) passes it as a
        // constant. Survey is now in `CALLABLE_FRAGMENTS` and `registry.rs` declares `Serves` at
        // all three doors.
        //
        // Re-pointed rather than deleted, same reason the `substantiate` test above was: had this
        // been removed when it went red, the witness that survey stopped refusing for a GOOD reason
        // (the `p_lens` settlement + the wrapper) would have retired. The assertion follows the act
        // out of the unreachable set: survey with an intention validates CLEAN.
        //
        // The plan needs an intention on the survey stage, because survey requires one (same as
        // the find acts). Without a query, survey collapses into `cogmap_read(shape)`, which
        // already serves pure orientation.
        let mut node = act("hits", ActName::Survey, Some(caller_ids(IdKind::Cogmap)));
        if let StageNode::Act(a) = &mut node {
            a.intention = Some(Intention {
                query: "salience".to_string(),
                embedding: Some(vec![0.1; 4]),
            });
        }
        let c = plan(vec![node], vec!["hits"]);
        validate(&c).expect("survey with an intention must validate clean now");
    }

    /// The other half of the flip above: **`follow-from` no longer refuses**, and the reason it
    /// stopped is the one the map records rather than a general loosening `[2026-08-14]`.
    ///
    /// Without this, the test above could be made to pass by deleting an act from a list, and
    /// nothing would notice that the act had become reachable for a bad reason — or not at all.
    #[test]
    fn a_predicate_on_the_invocation_is_redirected_to_both_containers() {
        // **The `properties` refusal is now ONE arm** `[2026-08-15]`. It was two — an edge arm that
        // redirected and a resource arm that flatly declined — and the pair existed because the
        // arms retired independently. They have both retired now, so a split would be a
        // distinction with nothing behind it.
        //
        // The predecessor test predicted exactly this: *"If a later edit gives this arm a redirect
        // too, that is a capability claim and this test is what makes it a deliberate one."* It is
        // a capability claim, and it is deliberate — `ResourceFilter::properties` applies through
        // `__temper_ungated_find_resources_with`'s `p_properties` (`20260815000040`).
        //
        // Asserted on the MESSAGE rather than only the reason, because the reason
        // (`FilterNotApplicable`) is the same one the flat decline gave: a test reading only the
        // discriminant cannot tell a redirect from a dead end, and would pass unchanged if this
        // arm were reverted.
        let c = plan_with_property("confidence", PropertyOp::HasKey);
        let errs = validate(&c).expect_err("still refused");
        let detail = errs
            .iter()
            .find(|e| e.reason == RefusalReason::FilterNotApplicable)
            .map(|e| e.detail.clone())
            .expect("a FilterNotApplicable refusal");
        // Both containers, because the subject tag that would have picked one is deleted.
        assert!(
            detail.contains("resource_filter.properties")
                && detail.contains("edge_filter.properties"),
            "the refusal must name both containers; got {detail}"
        );
        assert!(
            detail.contains("confidence"),
            "and name the predicate it is redirecting; got {detail}"
        );
        // The flat decline is gone, not merely reworded. It said the door "does not yet apply
        // property predicates", which is now false on both halves — a refusal outliving the
        // limitation it describes is worse than no refusal, because it teaches a caller that a
        // capability is missing when it is there.
        assert!(
            !detail.contains("does not yet apply"),
            "the flat decline outlived the limitation it described; got {detail}"
        );
    }

    #[test]
    fn the_selection_act_admits_a_property_predicate_in_its_resource_filter() {
        // The capability this half adds, stated as the ABSENCE of a refusal — the container is
        // reachable through `validate`, which is the layer that used to refuse it outright. Sixty-
        // seven of the seventy live property keys had no narrowing on any act before this.
        let c = selection_with_properties(vec![PropertyPredicate {
            key: "derived_from".to_string(),
            op: PropertyOp::Contains {
                values: vec![serde_json::json!("spec-a")],
            },
        }]);
        assert!(
            validate(&c).is_ok(),
            "the open-key container must be admitted on the act that applies it: {:?}",
            validate(&c).err()
        );
    }

    #[test]
    fn an_open_key_predicate_is_still_refused_on_an_act_that_does_not_select() {
        // The container is a parameter of ONE act. Carried on a find act it would narrow nothing,
        // and a narrowing that narrows nothing is the silent substitution this contract exists to
        // remove — so it is declined by name, pointing at the act that does apply it.
        let mut node = act("s", ActName::FindExact, Some(caller_ids(IdKind::Resource)));
        if let StageNode::Act(a) = &mut node {
            a.resource_filter = Some(ResourceFilter {
                properties: vec![PropertyPredicate {
                    key: "derived_from".to_string(),
                    op: PropertyOp::HasKey,
                }],
                ..Default::default()
            });
        }
        let c = plan_with_intention(vec![node], vec!["s"]);
        let errs = validate(&c).expect_err("a find act does not select");
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::FilterNotApplicable
                    && e.detail.contains("`properties`")
                    && e.detail.contains("find-resources-with")),
            "the refusal must name the field and the act that applies it; got {errs:?}"
        );
    }

    #[test]
    fn the_act_that_walks_edges_admits_a_property_predicate_in_its_edge_filter() {
        // The capability this change adds, stated as the absence of a refusal — the container is
        // reachable through `validate`, which is the layer that used to refuse it unconditionally.
        //
        // **This asserts reachability, NOT that the predicate narrows anything.** The narrowing is
        // witnessed against a real database, by a test that CREATES an edge-owned property, because
        // zero of them exist in this deployment and a wire-level green here would prove only that
        // the plan validates.
        let mut node = act("s", ActName::FollowFrom, None);
        if let StageNode::Act(a) = &mut node {
            a.edge_filter = Some(EdgeFilter {
                edge_kinds: vec![],
                labels: vec![],
                properties: vec![PropertyPredicate {
                    key: "confidence".to_string(),
                    op: PropertyOp::Contains {
                        values: vec![serde_json::json!("high")],
                    },
                }],
            });
        }
        if let Err(errs) = validate(&plan_with_intention(vec![node], vec!["s"])) {
            assert!(
                !errs
                    .iter()
                    .any(|e| e.reason == RefusalReason::FilterNotApplicable),
                "an edge property predicate on the one act that traverses edges is admitted; \
                 got {errs:?}"
            );
        }
    }

    #[test]
    fn a_walk_caps_the_edge_property_predicates_that_multiply_per_edge_cost() {
        // Same cost shape as `facets`, so the same cap — spec §9 says it "becomes theirs". Each
        // predicate is a nested `NOT EXISTS` evaluated once per candidate EDGE, so the list is a
        // multiplier an authenticated caller chooses. `edge_kinds` and `labels` are `= ANY` and one
        // operation whatever their length, which is why neither is capped and this is not.
        let over = |n: usize| {
            let mut node = act("s", ActName::FollowFrom, None);
            if let StageNode::Act(a) = &mut node {
                a.edge_filter = Some(EdgeFilter {
                    edge_kinds: vec![],
                    labels: vec![],
                    properties: (0..n)
                        .map(|i| PropertyPredicate {
                            key: format!("k{i}"),
                            op: PropertyOp::HasKey,
                        })
                        .collect(),
                });
            }
            validate(&plan_with_intention(vec![node], vec!["s"]))
        };
        // The bite probe: one under the cap must NOT refuse, or the assertion below would pass
        // against a rule that refuses everything.
        assert!(
            over(32).is_ok() || !refuses_with(&over(32), "per-edge predicates"),
            "32 is admitted"
        );
        assert!(
            refuses_with(&over(33), "at most 32 per-edge predicates"),
            "33 is refused"
        );
        // **The refusal still says WHICH container refused** `[2026-08-15]`. Both caps render from
        // one body now, and the failure mode of parameterizing a message is a shared string that
        // refuses correctly and leaves a caller with two containers on one stage unable to tell
        // which to fix. `walk` and `edge` are the two nouns that carry it.
        assert!(
            refuses_with(&over(33), "a walk admits")
                && refuses_with(&over(33), "every candidate edge"),
            "the edge cap names the walk and its candidate, rather than refusing in a voice the \
             resource container could also have spoken"
        );
        // No facet breakdown: `EdgeFilter` has no facet axis, so rendering "(0 facet, N open-key)"
        // would offer a caller a field that does not exist on the container that refused.
        assert!(
            !refuses_with(&over(33), "facet"),
            "the edge refusal names no facet count"
        );
    }

    /// Whether a validation outcome carries a `FilterNotApplicable` whose detail contains `needle`.
    fn refuses_with<T>(out: &Result<T, Vec<PlanRefusal>>, needle: &str) -> bool {
        out.as_ref().err().is_some_and(|errs| {
            errs.iter().any(|e| {
                e.reason == RefusalReason::FilterNotApplicable && e.detail.contains(needle)
            })
        })
    }

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
        //
        // `[amended — 2026-08-16]` Survey now produces RESOURCES (the ratified ⟨3⟩ redesign), not
        // regions. The `score_kind`/`orders_by`/`scale` pairing still holds — `region_score` is
        // still the quantity, still `OtherRange`, still named by both accessors. What changed is
        // `produced_variant`: it is now `Resources`, because survey returns resource rows. The
        // `Regions` variant of `ProducedVariant` is now unexercised by any declaration — kept
        // rather than deleted, because a future act that produces regions would need it.
        let survey = declaration(&ActName::Survey).expect("survey is declared");
        assert_eq!(
            survey.score_kind().as_ref().map(ScoreKind::as_str),
            survey.orders_by.as_ref().map(|q| q.field.as_str())
        );
        assert!(matches!(
            survey.orders_by.as_ref().map(|q| &q.scale),
            Some(crate::types::query::QuantityScale::OtherRange { .. })
        ));
        assert_eq!(survey.produced_variant(), Some(ProducedVariant::Resources));
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

    /// `A − B` — the shape the op exists for, and the only arity it admits.
    fn difference(name: &str, minuend: &str, subtrahend: &str) -> StageNode {
        StageNode::Combine(CombineNode {
            name: StageName::parse(name).unwrap(),
            op: CombineOp::Difference,
            inputs: vec![
                StageName::parse(minuend).unwrap(),
                StageName::parse(subtrahend).unwrap(),
            ],
        })
    }

    #[test]
    fn a_two_input_difference_is_admitted() {
        // The boundary the refusal below is measured against. Without this, an arity rule that
        // refused EVERY difference would look identical from the failing side.
        let c = plan_with_intention(
            vec![
                act("a", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
                act("b", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
                difference("gap", "a", "b"),
            ],
            vec!["a"],
        );
        assert!(validate(&c).is_ok(), "got: {:?}", validate(&c).unwrap_err());
    }

    #[test]
    fn a_three_input_difference_is_refused_because_the_union_it_folds_would_have_no_stage() {
        // Postgres would evaluate `a EXCEPT b EXCEPT c` as `a − (b ∪ c)` — the exact question this
        // op exists for, one stage cheaper. It is refused anyway: the fold gives `b ∪ c` no stage,
        // so nothing tallies it and no reader can see how large the set that did the subtracting
        // was. Declaring the union as its own stage is what makes the narrowing legible.
        //
        // Asserted as a REFUSAL rather than a truncation: silently dropping the third input would
        // answer a narrower question than the one asked.
        let c = plan_with_intention(
            vec![
                act("a", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
                act("b", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
                act("c", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
                StageNode::Combine(CombineNode {
                    name: StageName::parse("gap").unwrap(),
                    op: CombineOp::Difference,
                    inputs: vec![
                        StageName::parse("a").unwrap(),
                        StageName::parse("b").unwrap(),
                        StageName::parse("c").unwrap(),
                    ],
                }),
            ],
            vec!["a"],
        );
        let errs = validate(&c).expect_err("a difference takes exactly two inputs");
        let hit = errs
            .iter()
            .find(|e| {
                e.reason == RefusalReason::CombinatorArity
                    && e.stage.as_ref().is_some_and(|s| s.as_str() == "gap")
            })
            .unwrap_or_else(|| panic!("got: {errs:?}"));
        // The refusal has to name the repair, not just the rule — the caller's question IS
        // expressible and the union stage is the whole of what they are missing.
        assert!(
            hit.detail.contains("union"),
            "a refusal that does not name the route is a dead end: {}",
            hit.detail
        );
    }

    #[test]
    fn a_difference_produces_its_minuends_kind_and_never_its_subtrahends() {
        // `produced_kind_of` walks a combinator to `inputs.first()`. For a union or an intersect
        // that is arbitrary-but-consistent — mixed kinds are a malformed plan either way. For a
        // difference it is LOAD-BEARING: the stage produces a subset of its minuend, so consulting
        // the subtrahend would attribute the wrong kind to a perfectly well-formed plan.
        //
        // `[amended — 2026-08-16]` The two arms used to differ in kind (survey produced `Region`,
        // find-exact produced `Resource`), which was what made the assertion bite — flip
        // `produced_kind_of` to `.last()` and `UnsupportedSeedKind` would appear. Survey now
        // produces `Resource` (the ratified ⟨3⟩ redesign), so both arms are `Resource` and the
        // difference is resource−resource. The test validates clean rather than asserting the
        // ABSENCE of a seed-kind refusal. The `produced_kind_of` invariant is still witnessed by
        // this plan: `FollowFrom` accepts `Resource` seeds, the difference produces `Resource`
        // (the minuend's kind), and `validate` accepts it.
        let c = plan_with_intention(
            vec![
                act(
                    "tasks",
                    ActName::FindExact,
                    Some(caller_ids(IdKind::Resource)),
                ),
                act("regions", ActName::Survey, Some(caller_ids(IdKind::Cogmap))),
                difference("gap", "tasks", "regions"),
                act(
                    "reached",
                    ActName::FollowFrom,
                    Some(StageInput::Upstream {
                        relation: StageRelation::Seed,
                        stage: StageName::parse("gap").unwrap(),
                    }),
                ),
            ],
            vec!["reached"],
        );
        validate(&c).expect("the plan validates — both arms produce resources, the difference is resource−resource, and FollowFrom accepts resource seeds");
    }

    /// **The question this op was added for, written out and validated end to end.**
    ///
    /// *"Which tasks advancing this goal declare neither `open_meta.witnesses` nor
    /// `open_meta.enables`"* — `A − (B ∪ C)`, the shape
    /// [Dogfood: register coverage is a composition](./01a0002b-9fd1-78c3-b1e4-7e3400e9b5d0) names.
    ///
    /// It is here rather than in an execute test because what it witnesses is EXPRESSIBILITY, and
    /// that is decidable without a corpus: every refusal in this contract but one is static, so a
    /// plan that validates is a plan the compiler will emit.
    ///
    /// # The shape below is the SECOND one, and the first was expressible and answered nothing
    ///
    /// `[falsified against prod — 2026-08-15]` This test first bounded the WALK by the difference —
    /// `follow-from(seed = the goal, bound = gap)` — on the reasoning that
    /// [`crate::types::query::registry`]'s note says the bound is *"applied where visibility is
    /// applied so it constrains INTERMEDIATE nodes and not merely the returned set"*, which also
    /// absorbs the act's fixed depth of 2.
    ///
    /// **That plan validates, compiles, runs, and returns zero rows for a structural reason.**
    /// `__temper_ungated_follow_from` seeds only from ids that survive the bound —
    /// `FROM unnest(p_seed_ids) AS s(id) WHERE EXISTS (SELECT 1 FROM admitted a WHERE a.id = s.id)`,
    /// where `admitted` is `visible ∩ p_bound_ids`. The seed here is a **goal**; the bound is a set
    /// of **tasks**; a goal is never in it, so the walk never starts. The doc note is true and was
    /// over-read: the bound constrains intermediate nodes *and the seed*.
    ///
    /// **Nothing in this crate could have caught it**, and that is the lesson rather than the bug.
    /// `validate` is static — it answers *"is this expressible"*, and the wrong plan is perfectly
    /// expressible. Only running it against a corpus distinguishes the two, which is exactly the
    /// gap between this test and the task's own third acceptance criterion.
    ///
    /// So the walk runs UNBOUNDED and the narrowing happens in set operations after it — which is
    /// the shape the task originally proposed, and it was right. The act's fixed depth of 2 is
    /// absorbed by `∩ tasks`: measured on prod `[2026-08-15]`, hop 2 adds exactly one node and no
    /// additional task.
    ///
    /// # What this still catches that `a_two_input_difference_is_admitted` cannot
    ///
    /// **The difference cannot be the returned stage.** A combinator's rows have no single act to
    /// score them (`StageNotReturnable`), so the answer's stage cannot be asked for directly — the
    /// composition returns the walk, and `gap`'s size is read from its trace.
    ///
    /// `[amended — 2026-08-15]` This read *"[`crate::types::query::trace::StageTrace`] carries no
    /// produced count, so a terminal combinator's row count is otherwise recoverable only from a
    /// downstream stage's `input_ids`, and `gap` has no downstream. Its `narrowed_by` disclosure is
    /// what makes the answer readable at all."* **The first clause is now false**:
    /// [`crate::types::query::trace::StageTrace::produced_ids`] carries it for every stage, which
    /// is what that sentence was evidence for the absence of.
    ///
    /// What survives is the part that was never about the count. `narrowed_by` on a difference
    /// names **which arm was the subtrahend** — a combinator contributes no per-input entries, so
    /// `a − b` and `b − a` are otherwise the same trace — and reports `excluded` as
    /// `|minuend| − |result|` rather than `|subtrahend|`, an arithmetic a reader holding three raw
    /// counts can get wrong in the flattering direction. So the two disclosures are not redundant:
    /// this one answers *how many*, that one answers *what narrowed what*.
    #[test]
    fn the_register_coverage_question_is_expressible_as_a_composition() {
        let selection = |name: &str, filter: ResourceFilter| {
            StageNode::Act(ActInvocation {
                name: StageName::parse(name).unwrap(),
                act: ActName::FindResourcesWith,
                intention: None,
                inputs: vec![],
                terms: BTreeMap::new(),
                resource_filter: Some(filter),
                edge_filter: None,
                properties: vec![],
            })
        };
        let has_key = |key: &str| ResourceFilter {
            properties: vec![PropertyPredicate {
                key: key.to_string(),
                op: PropertyOp::HasKey,
            }],
            ..Default::default()
        };
        let combine = |name: &str, op: CombineOp, a: &str, b: &str| {
            StageNode::Combine(CombineNode {
                name: StageName::parse(name).unwrap(),
                op,
                inputs: vec![StageName::parse(a).unwrap(), StageName::parse(b).unwrap()],
            })
        };

        let compose = |returned: &str| Composition {
            outcome: OutcomeDeclaration {
                returns: vec![ReturnSpec {
                    stage: StageName::parse(returned).unwrap(),
                    with: vec![],
                }],
            },
            stages: vec![
                // UNBOUNDED, per the falsification above: a bounded walk whose seed is outside the
                // bound never starts.
                StageNode::Act(ActInvocation {
                    name: StageName::parse("advancing").unwrap(),
                    act: ActName::FollowFrom,
                    intention: None,
                    inputs: vec![StageInput::Caller {
                        relation: StageRelation::Seed,
                        ids: IdSet {
                            kind: IdKind::Resource,
                            provenance: None,
                            ids: vec![uuid::Uuid::now_v7()],
                        },
                    }],
                    terms: BTreeMap::new(),
                    resource_filter: None,
                    edge_filter: Some(EdgeFilter {
                        labels: vec!["advances".to_string()],
                        ..Default::default()
                    }),
                    properties: vec![],
                }),
                selection(
                    "tasks",
                    ResourceFilter {
                        doc_type: vec!["task".to_string()],
                        ..Default::default()
                    },
                ),
                // Absorbs both the non-task advancers (4 of 35, measured) and the depth-2 arrivals.
                combine(
                    "advancing_tasks",
                    CombineOp::Intersect,
                    "advancing",
                    "tasks",
                ),
                selection("witnessing", has_key("witnesses")),
                selection("enabling", has_key("enables")),
                combine("declared", CombineOp::Union, "witnessing", "enabling"),
                combine("gap", CombineOp::Difference, "advancing_tasks", "declared"),
            ],
        };

        validate(&compose("advancing"))
            .unwrap_or_else(|e| panic!("the question must be expressible; got {e:?}"));

        // **The denominator.** Without this, the assertion above is "some composition validates",
        // which a plan with the difference stage deleted would satisfy just as well. The SAME plan
        // asking for the answer's rows back is refused — which is why the count has to be read from
        // the trace, and why the subtraction disclosure exists.
        let errs = validate(&compose("gap")).expect_err("a combinator's rows have no scorer");
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::StageNotReturnable
                    && e.stage.as_ref().is_some_and(|s| s.as_str() == "gap")),
            "got: {errs:?}"
        );
    }

    #[test]
    fn a_three_input_union_stays_legal_so_the_arity_rule_is_per_op() {
        // The other half: the ceiling belongs to `difference` alone. A rule written on
        // `CombineNode` rather than on the op would take the three-way union with it, and nothing
        // else in the contract would have said so.
        let c = plan_with_intention(
            vec![
                act("a", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
                act("b", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
                act("c", ActName::FindExact, Some(caller_ids(IdKind::Resource))),
                StageNode::Combine(CombineNode {
                    name: StageName::parse("merged").unwrap(),
                    op: CombineOp::Union,
                    inputs: vec![
                        StageName::parse("a").unwrap(),
                        StageName::parse("b").unwrap(),
                        StageName::parse("c").unwrap(),
                    ],
                }),
            ],
            vec!["a"],
        );
        assert!(validate(&c).is_ok(), "got: {:?}", validate(&c).unwrap_err());
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
