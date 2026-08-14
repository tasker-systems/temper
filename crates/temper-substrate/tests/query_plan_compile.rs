//! Pure emission tests for the query compiler — NO database. The security property (every caller
//! value is bound, never interpolated) and the structural properties (one visibility relation, ids
//! only across a stage boundary, dependency order) are all decidable from the emitted text alone.

use temper_core::types::ids::ProfileId;
use temper_core::types::query::{
    declaration, validate, ActInvocation, ActName, Composition, IdKind, IdSet, OutcomeDeclaration,
    ReturnSpec, StageInput, StageName, StageNode, StageRelation, ValidatedComposition,
};
use temper_substrate::readback::query_plan::{compile, QueryBind};
use uuid::Uuid;

fn test_profile() -> ProfileId {
    ProfileId::new()
}

/// A `find-exact` root bounded by a caller resource set.
///
/// `[was `ff_root`, a `follow-from` root — 2026-08-12]` `follow-from` and `survey` left
/// `CALLABLE_FRAGMENTS`, so `validate` now refuses them and no plan built from one can reach
/// `compile` at all. The structural properties below — every value bound, ids only across a
/// boundary, one visibility relation, dependency order — are about the STATEMENT and not about
/// which act produced it, so they move onto a reachable act unchanged.
fn find_root(name: &str, ids: Vec<Uuid>) -> StageNode {
    StageNode::Act(ActInvocation {
        name: StageName::parse(name).unwrap(),
        act: ActName::FindExact,
        intention: Some(Intention {
            query: "salience".to_string(),
            embedding: None,
        }),
        inputs: vec![StageInput::Caller {
            relation: StageRelation::Bound,
            ids: IdSet {
                kind: IdKind::Resource,
                provenance: None,
                ids,
            },
        }],
        terms: Default::default(),
        resource_filter: None,
        edge_filter: None,
        properties: vec![],
    })
}

/// A `find-exact` stage bounded by an upstream stage.
///
/// `find-exact` rather than `find-about-within` for the downstream half deliberately: the wide arm
/// needs an embedding, and a stage the compiler refuses for want of one emits a refused body
/// instead of the bound this file's assertions read.
fn find_from(name: &str, upstream: &str) -> StageNode {
    StageNode::Act(ActInvocation {
        name: StageName::parse(name).unwrap(),
        act: ActName::FindExact,
        intention: Some(Intention {
            query: "salience".to_string(),
            embedding: None,
        }),
        inputs: vec![StageInput::Upstream {
            relation: StageRelation::Bound,
            stage: StageName::parse(upstream).unwrap(),
        }],
        terms: Default::default(),
        resource_filter: None,
        edge_filter: None,
        properties: vec![],
    })
}

/// A validated composition with the question threaded — which every find act requires, and which is
/// now every act this surface can emit.
///
/// `[merged with `build_with_intention` — 2026-08-12]` The two builders differed only in whether
/// they threaded an intention, and the intention-free one existed for `follow-from` plans. There are
/// none left, and two identical builders are two things that can drift.
fn build(stages: Vec<StageNode>, returns: Vec<&str>) -> ValidatedComposition {
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: returns
                .into_iter()
                .map(|s| ReturnSpec {
                    stage: StageName::parse(s).unwrap(),
                    with: vec![],
                })
                .collect(),
        },
        stages,
    };
    validate(&c).expect("plan is valid")
}

fn plan_with_caller_ids() -> (ValidatedComposition, Vec<Uuid>) {
    let ids = vec![Uuid::now_v7(), Uuid::now_v7()];
    (
        build(vec![find_root("hits", ids.clone())], vec!["hits"]),
        ids,
    )
}

fn plan_two_stages(root: &str, downstream: &str) -> ValidatedComposition {
    build(
        vec![
            find_root(root, vec![Uuid::now_v7()]),
            find_from(downstream, root),
        ],
        vec![downstream],
    )
}

fn plan_two_stages_declared_backwards(root: &str, downstream: &str) -> ValidatedComposition {
    // The downstream stage is DECLARED first; validate()'s topological order must still put the
    // root before it, and the compiler must emit in that order.
    build(
        vec![
            find_from(downstream, root),
            find_root(root, vec![Uuid::now_v7()]),
        ],
        vec![downstream],
    )
}

fn plan_one_stage() -> ValidatedComposition {
    build(vec![find_root("hits", vec![Uuid::now_v7()])], vec!["hits"])
}

/// One `find-exact` stage with NO input — the unbounded shape.
///
/// Post-flip this differs from [`plan_one_stage`] only by that input, and the honest reason both
/// survive is that they have different consumers: the compute-once test takes this one and
/// `no_stage_can_be_named_after_the_hoisted_visibility_relation` takes the other. **Not** a claimed
/// coverage relationship — nothing here asserts the compute-once property over the bounded shape,
/// and the earlier version of this comment said otherwise. Before the flip the distinction was
/// load-bearing (`plan_one_stage` was a `follow-from` plan that emitted the placeholder and
/// consulted no visibility relation, so counting gate calls over it proved nothing); it is not
/// load-bearing now, and either helper would serve either test.
fn plan_one_find() -> ValidatedComposition {
    build(vec![find_stage("a", ActName::FindExact, None)], vec!["a"])
}

/// Three chained `find-exact` stages. `find-exact` throughout rather than a `find-about-*` mix so
/// the plan compiles with no embedding; the property under test is about the gate, not the arm.
fn plan_three_finds() -> ValidatedComposition {
    build(
        vec![
            find_stage("a", ActName::FindExact, None),
            find_stage(
                "b",
                ActName::FindExact,
                Some(StageInput::Upstream {
                    relation: StageRelation::Bound,
                    stage: StageName::parse("a").unwrap(),
                }),
            ),
            find_stage(
                "c",
                ActName::FindExact,
                Some(StageInput::Upstream {
                    relation: StageRelation::Bound,
                    stage: StageName::parse("b").unwrap(),
                }),
            ),
        ],
        vec!["c"],
    )
}

// `plan_three_stages` (three chained `follow-from` stages) lived here and was deleted rather than
// `#[allow(dead_code)]`d when its only consumer, the compute-once test, moved to `plan_three_finds`:
// a `follow-from` stage emitted the placeholder and consulted no visibility relation at all, so a
// three-stage plan built from those satisfied the assertion while proving nothing. Rust noticing it
// went unused is the check working. (`follow-from` no longer validates at all, so that plan is not
// merely unhelpful now — it is unbuildable.)

#[test]
fn every_caller_value_is_bound_and_none_is_interpolated() {
    // The security property, tested where it can be tested exhaustively: no database, no fixtures,
    // just the emitted text. A uuid appearing literally in the SQL is the failure.
    let (v, ids) = plan_with_caller_ids();
    let c = compile(&v, test_profile()).expect("compiles");
    for id in ids {
        assert!(
            !c.sql.contains(&id.to_string()),
            "id {id} was interpolated, not bound"
        );
    }
    assert!(c.binds.iter().any(|b| matches!(b, QueryBind::Uuids(_))));
}

#[test]
fn the_only_identifiers_emitted_are_validated_stage_names() {
    // StageName::parse is the gate (beat A). This asserts the emitter honours it rather than
    // formatting arbitrary strings into identifier position.
    let v = plan_two_stages("hits", "near");
    let c = compile(&v, test_profile()).expect("compiles");
    assert!(c.sql.contains(r#""hits" AS ("#));
    assert!(c.sql.contains(r#""near" AS ("#));
    assert_eq!(c.cte_names.len(), 2);
}

#[test]
fn the_visibility_relation_is_computed_once_no_matter_how_many_stages() {
    // Decision 019fcd13: one query time, one visibility computation. A per-stage recomputation is
    // the thing the single statement exists to collapse.
    //
    // **This assertion used to be vacuous and said so.** Its predecessor counted the CTE's TEXT,
    // which proved only that the compiler emitted it once — while every act body called a twin that
    // gated internally, so nothing read the CTE and an N-stage composition paid N full gates,
    // including N recursive team closures (the planner does not dedupe gate calls across call
    // sites: a STABLE function permits caching within a scan, not common-subexpression
    // elimination). It carried a companion assertion that no stage consumed `vis` yet, whose
    // failure was the agreed signal to write this.
    //
    // The real property is countable without a database and is asserted here: `resources_visible_to`
    // appears EXACTLY ONCE in the emitted statement, at any stage count. The ungated cores
    // (`20260808000030`) do not call it, so any second occurrence means a stage went back to gating
    // for itself.
    let one = compile(&plan_one_find(), test_profile()).expect("compiles");
    let three = compile(&plan_three_finds(), test_profile()).expect("compiles");

    assert_eq!(
        one.sql.matches("resources_visible_to(").count(),
        1,
        "got:\n{}",
        one.sql
    );
    assert_eq!(
        three.sql.matches("resources_visible_to(").count(),
        1,
        "three stages must still evaluate the gate once; got:\n{}",
        three.sql
    );
    assert_eq!(three.sql.matches("__temper_vis AS MATERIALIZED").count(), 1);

    // And it is READ — the half whose absence made the old test vacuous. Emitting a relation nobody
    // consumes is worse than not emitting it, because `MATERIALIZED` computes it anyway.
    assert_eq!(
        three.sql.matches("FROM __temper_vis").count(),
        3,
        "every act stage must take its verdict from the hoisted relation; got:\n{}",
        three.sql
    );
}

#[test]
fn a_downstream_stage_selects_ids_only_and_never_a_quantity() {
    // THE rule that keeps no-cross-act-ranking structural (spec §4). If a quantity can cross a
    // stage boundary, cross-act arithmetic becomes mechanically easy and nothing prevents it.
    let v = plan_two_stages("hits", "near");
    let c = compile(&v, test_profile()).expect("compiles");
    let downstream = c
        .sql
        .split(r#""near" AS ("#)
        .nth(1)
        .expect("near CTE present");
    assert!(
        downstream.contains(r#"SELECT id FROM "hits""#),
        "got: {downstream}"
    );
    assert!(
        !downstream.contains(r#"quantity FROM "hits""#),
        "a quantity crossed a stage boundary"
    );
}

// ─── The tallies the executor reads to fill the trace ───────────────────────────────────────────
//
// The trace covers EVERY stage, including the ones whose rows are not returned — that is what lets
// a reader see whether stage 2 earned its place. Those stages have no rows in the result, so the
// only thing that can tell the executor a non-returned stage was `empty` rather than `answered` is
// a count travelling in the SAME statement. A second query would answer from a different snapshot,
// which is the property `one statement, one snapshot` exists to hold.

/// The one tally arm for `stage`, isolated from the rest of the statement.
///
/// Split on the tally's own row-class marker rather than on the stage label alone: a RETURNED
/// stage labels both a hit arm and a tally arm, so matching the label would silently hand back the
/// hit arm and let an assertion about the tally pass without ever reading one.
fn tally_arm(sql: &str, stage: &str) -> String {
    let marker = format!("'tally'::text AS row_class, '{stage}'::text AS stage");
    let rest = sql
        .split(&marker)
        .nth(1)
        .unwrap_or_else(|| panic!("no tally arm for `{stage}` in:\n{sql}"));
    rest.split("UNION ALL").next().unwrap().to_string()
}

fn hit_arm(sql: &str, stage: &str) -> Option<String> {
    let marker = format!("'hit'::text AS row_class, '{stage}'::text AS stage");
    let rest = sql.split(&marker).nth(1)?;
    Some(rest.split("UNION ALL").next().unwrap().to_string())
}

#[test]
fn every_stage_is_tallied_including_one_whose_rows_are_not_returned() {
    let v = plan_two_stages("hits", "near");
    let c = compile(&v, test_profile()).expect("compiles");
    // `near` is the only returned stage; `hits` feeds it and reaches the caller only as a trace
    // entry — which it cannot have without a count.
    tally_arm(&c.sql, "hits");
    tally_arm(&c.sql, "near");
    assert!(hit_arm(&c.sql, "near").is_some(), "got: {}", c.sql);
    assert!(
        hit_arm(&c.sql, "hits").is_none(),
        "a stage nobody asked for must not ship its rows: {}",
        c.sql
    );
}

#[test]
fn a_tally_row_carries_no_id_so_a_non_returned_stages_rows_never_leave_the_database() {
    // The tally discloses HOW MANY, never WHICH. A non-returned stage's ids are the pipe's internal
    // currency, and shipping them would hand back rows the caller did not ask for.
    let v = plan_two_stages("hits", "near");
    let c = compile(&v, test_profile()).expect("compiles");
    let tally = tally_arm(&c.sql, "hits");
    assert!(
        tally.contains("NULL::uuid"),
        "a tally carries no id: {tally}"
    );
    assert!(
        tally.contains(r#"count(*) FROM "hits""#),
        "and it does carry the count: {tally}"
    );
}

#[test]
fn a_caller_supplied_set_is_tallied_against_the_hoisted_visibility_relation() {
    // `input_unusable` — invisible, nonexistent and malformed as ONE number. It is computable only
    // where the ids came from the CALLER, and it is counted against the relation that already
    // exists rather than by asking the gate a second time.
    let (v, _ids) = plan_with_caller_ids();
    let c = compile(&v, test_profile()).expect("compiles");
    let tally = tally_arm(&c.sql, "hits");
    assert!(
        tally.contains("unnest(") && tally.contains("__temper_vis"),
        "the supplied set is counted against the ONE visibility relation: {tally}"
    );
    assert_eq!(
        c.sql.matches("resources_visible_to").count(),
        1,
        "tallying must not add a second gate call: {}",
        c.sql
    );
}

#[test]
fn an_upstream_fed_stage_tallies_zero_unusable_rather_than_re_gating_it() {
    // Not a shortcut: an upstream set is what a visibility-gated fragment returned, so every id in
    // it was usable by construction. Re-checking would cost a gate call to confirm a known answer.
    let v = plan_two_stages("hits", "near");
    let c = compile(&v, test_profile()).expect("compiles");
    let tally = tally_arm(&c.sql, "near");
    assert!(
        !tally.contains("unnest("),
        "an upstream-fed stage re-gated its input: {tally}"
    );
    assert!(
        tally.contains("0::bigint"),
        "and says so as a literal zero rather than leaving it null: {tally}"
    );
}

/// **A combinator projects membership only — a quantity must never enter a set operation.**
///
/// `UNION` and `INTERSECT` compare WHOLE ROWS. With `quantity` in the projection, a resource found
/// by two acts carries two different scores (`fts_norm` from the exact arm, `vec_norm` from the
/// wide one) and is therefore two distinct rows: `intersect` across two acts was ALWAYS empty and
/// `union` counted the resource twice. Found in review.
///
/// Asserted here rather than end-to-end because the execute-level version needs two acts whose
/// scores differ for one resource, which needs embedded chunk fixtures — and because this is where
/// the rule lives. It is also the stage contract restated: a quantity never crosses a stage
/// boundary, and a set operation is the clearest case of that.
#[test]
fn a_combinator_projects_membership_only_and_never_a_quantity() {
    let v = build(
        vec![
            find_root("a", vec![Uuid::now_v7()]),
            find_root("b", vec![Uuid::now_v7()]),
            StageNode::Combine(temper_core::types::query::CombineNode {
                name: StageName::parse("merged").unwrap(),
                op: temper_core::types::query::CombineOp::Intersect,
                inputs: vec![
                    StageName::parse("a").unwrap(),
                    StageName::parse("b").unwrap(),
                ],
            }),
        ],
        vec!["a"],
    );
    let c = compile(&v, test_profile()).expect("compiles");
    let body = c
        .sql
        .split(r#""merged" AS ("#)
        .nth(1)
        .expect("merged CTE")
        .split("\n)")
        .next()
        .unwrap()
        .to_string();

    assert!(body.contains("INTERSECT"), "got: {body}");
    assert!(
        !body.contains("quantity"),
        "a quantity in a set operation makes one resource two rows: {body}"
    );
    assert!(
        body.contains(r#"SELECT id, kind FROM "a""#),
        "membership only, both arms: {body}"
    );
}

#[test]
fn stages_are_emitted_in_dependency_order() {
    // The compiler consumes ValidatedComposition::ordered(); a CTE referencing one declared later
    // would not parse.
    let v = plan_two_stages_declared_backwards("hits", "near");
    let c = compile(&v, test_profile()).expect("compiles");
    let hits_at = c.sql.find(r#""hits" AS ("#).unwrap();
    let near_at = c.sql.find(r#""near" AS ("#).unwrap();
    assert!(hits_at < near_at);
}

// ─── Beat D: the find acts emit real fragments ──────────────────────────────────────────────────

use temper_core::types::graph::EdgeKind;
use temper_core::types::query::{BoundTerm, EdgeFilter, Intention, RefusalReason};

/// The ungated cores, restated here because they are private to the crate. A drift between these
/// and the compiler's own constants shows up as a failing assertion, which is the right failure.
const EMIT_FIND_EXACT: &str = "__temper_ungated_find_exact";
const EMIT_FIND_WIDE: &str = "__temper_ungated_find_wide";

/// A find stage carrying its own question. `[2026-08-12]` The question used to come from the
/// composition envelope; spec ⟨7⟩ put it on the stage, so the builder supplies it — otherwise every
/// plan here would refuse `MissingIntention` for a reason no test is about.
fn find_stage(name: &str, act: ActName, input: Option<StageInput>) -> StageNode {
    StageNode::Act(ActInvocation {
        name: StageName::parse(name).unwrap(),
        intention: Some(Intention {
            query: "salience".to_string(),
            embedding: None,
        }),
        act,
        inputs: input.into_iter().collect(),
        terms: Default::default(),
        resource_filter: None,
        edge_filter: None,
        properties: vec![],
    })
}

/// [`find_stage`] with no question at all — for the test that asserts `MissingIntention`, which is
/// now a property of the STAGE rather than of the composition.
fn find_stage_without_intention(name: &str, act: ActName, input: Option<StageInput>) -> StageNode {
    match find_stage(name, act, input) {
        StageNode::Act(mut inv) => {
            inv.intention = None;
            StageNode::Act(inv)
        }
        other => other,
    }
}

/// A find stage whose question carries a caller-supplied vector — the `find-about-*` case, where
/// the compiler binds the vector rather than refusing `EmbeddingUnavailable`.
fn find_stage_embedded(name: &str, act: ActName, input: Option<StageInput>) -> StageNode {
    match find_stage(name, act, input) {
        StageNode::Act(mut inv) => {
            inv.intention = Some(Intention {
                query: "salience".to_string(),
                embedding: Some(an_embedding()),
            });
            StageNode::Act(inv)
        }
        other => other,
    }
}

fn an_embedding() -> Vec<f32> {
    vec![0.25_f32; 768]
}

/// The pipe the whole phase exists for: a `find-about-within` stage narrowed by an UPSTREAM stage
/// compiles to a call on the composable twin, taking its bound from that stage's ids.
///
/// Fails against a builder that still emits `__temper_unbound_act`, and against one that emits
/// `search_wide` — which has no `p_bound_ids` and so cannot express this at all.
#[test]
fn a_find_about_within_stage_compiles_to_the_bounded_core_narrowed_by_its_upstream() {
    let v = build(
        vec![
            find_root("seeds", vec![Uuid::now_v7()]),
            find_stage_embedded(
                "narrowed",
                ActName::FindAboutWithin,
                Some(StageInput::Upstream {
                    relation: StageRelation::Bound,
                    stage: StageName::parse("seeds").unwrap(),
                }),
            ),
        ],
        vec!["narrowed"],
    );
    let c = compile(&v, test_profile()).expect("compiles");

    assert!(
        c.sql.contains("__temper_ungated_find_wide("),
        "the wide find act must target the ungated core — the twin gates internally, which is what \
         the hoisted relation exists to stop; got:\n{}",
        c.sql
    );
    assert!(
        !c.sql.contains("search_wide("),
        "it must NOT target the deployed arm, which has no p_bound_ids and cannot be bounded"
    );
    assert!(
        c.sql.contains(r#"ARRAY(SELECT id FROM "seeds")"#),
        "the bound must come from the upstream stage's ids; got:\n{}",
        c.sql
    );
    // Ids only across the boundary — no quantity of the upstream stage is ever in scope.
    assert!(
        !c.sql.contains("quantity FROM seeds"),
        "a quantity must never cross a stage boundary"
    );
}

/// The exact arm likewise, and its query text is BOUND rather than interpolated.
#[test]
fn a_find_exact_stage_binds_its_query_text_and_targets_the_exact_core() {
    let v = build(
        vec![find_stage("hits", ActName::FindExact, None)],
        vec!["hits"],
    );
    let c = compile(&v, test_profile()).expect("find-exact needs no embedding");

    assert!(
        c.sql.contains("__temper_ungated_find_exact("),
        "got:\n{}",
        c.sql
    );
    assert!(
        !c.sql.contains("salience"),
        "the query text must be a positional bind, never interpolated into the SQL"
    );
    assert!(
        c.binds
            .iter()
            .any(|b| matches!(b, QueryBind::Text(t) if t == "salience")),
        "the intention's query text must appear as a bind; got {:?}",
        c.binds
    );
    // An unbounded stage is NULL, never '{}' — the twins read those differently and conflating them
    // would turn "bounded to nothing" into "search everything".
    assert!(
        c.sql.contains("NULL::uuid[]"),
        "a stage with no input is UNBOUNDED, which is NULL and not an empty array; got:\n{}",
        c.sql
    );
}

/// **A `find-about-*` stage with no embedding REFUSES — as the SERVER's failure, not the caller's.**
///
/// The delta-2 property survives unchanged: a refusal is its own state, distinct from failure and
/// from honest-empty. Still fails against a builder that binds NULL for a missing embedding, which
/// would run a vector search on nothing and report an empty result that reads like an answer.
///
/// `[re-pointed — 2026-08-08, Pete]` What changed is WHICH refusal, and it is not cosmetic. This
/// asserted `MissingIntention`, encoding a rule that has since been reversed: that the caller must
/// supply the vector and the server never embeds on their behalf. The CLI can embed; the ruby gem,
/// the TypeScript package and MCP cannot, so that rule denied `find-about-*` to every non-CLI
/// client. The server embeds now, which means a `None` arriving here has already survived that
/// attempt — the caller did nothing wrong, and blaming their intention would be false.
///
/// Re-pointing rather than deleting, for the same reason `a_served_act_this_builder_has_no_fragment_for_refuses_honestly`
/// was re-pointed at beat D: the property still needs a witness, and deleting a test when its
/// expected value changes retires the only thing holding the distinction.
///
/// `[re-pointed again — 2026-08-09]` What changed this time is not WHICH refusal but WHERE it is
/// reported. It used to be an `Err`, which aborted the whole compilation — and the contract says
/// the opposite in as many words: *"Every other stage runs. Stages that do not depend on it are
/// unaffected and answer normally — a composition holding both a `find-exact` and a `find-about-*`
/// still returns the exact arm."* An `Err` loses the exact arm as well, which is a second stage
/// refused for a reason that has nothing to do with it. The refusal now rides on the compiled
/// query. The property under test is unchanged: a missing vector refuses rather than binding NULL.
#[test]
fn a_wide_find_without_an_embedding_refuses_as_the_servers_failure_not_the_callers() {
    let v = build(
        vec![find_stage("wide", ActName::FindAboutAnywhere, None)],
        vec!["wide"],
    );
    let c = compile(&v, test_profile()).expect("a runtime refusal does not abort the plan");

    let [refusal] = c.refusals.as_slice() else {
        panic!("exactly one stage refuses; got {:?}", c.refusals)
    };
    assert_eq!(refusal.reason, RefusalReason::EmbeddingUnavailable);
    assert_ne!(
        refusal.reason,
        RefusalReason::MissingIntention,
        "the composition threaded a question; the vector is the server's job, so reporting this \
         as a missing intention would blame the caller for the server's failure"
    );
    assert_eq!(refusal.stage.as_ref().map(|s| s.as_str()), Some("wide"));

    // Still not a NULL bind: the stage produces nothing rather than searching on nothing, which
    // would return a list that reads like an answer.
    assert!(
        !c.sql.contains(EMIT_FIND_WIDE),
        "a refused stage must not call the vector core at all: {}",
        c.sql
    );

    // And the SAME plan whose stage carries a vector compiles clean — so the refusal is about the
    // embedding and not about the plan being malformed in some other way. `[2026-08-12]` The
    // control used to re-compile `v` with the vector passed as an argument; the vector is on the
    // stage now, so the control differs from the subject in the intention and nowhere else.
    let with_vector = build(
        vec![find_stage_embedded(
            "wide",
            ActName::FindAboutAnywhere,
            None,
        )],
        vec!["wide"],
    );
    let ok = compile(&with_vector, test_profile()).expect("compiles");
    assert!(ok.refusals.is_empty());
}

/// **A refusal is per stage; the stages that do not depend on it answer normally.**
///
/// The contract's own worked case, and the reason the refusal cannot be an `Err`: a caller who asked
/// two independent questions and got one unanswerable vector must still be given the other answer.
#[test]
fn one_stage_refusing_for_want_of_an_embedding_does_not_refuse_the_others() {
    let v = build(
        vec![
            find_stage("exact", ActName::FindExact, None),
            find_stage("wide", ActName::FindAboutAnywhere, None),
        ],
        vec!["exact", "wide"],
    );
    let c = compile(&v, test_profile()).expect("compiles");

    assert_eq!(c.refusals.len(), 1, "only the wide stage refuses");
    assert_eq!(
        c.refusals[0].stage.as_ref().map(|s| s.as_str()),
        Some("wide")
    );
    assert!(
        c.sql.contains(EMIT_FIND_EXACT),
        "the exact arm still runs: {}",
        c.sql
    );
}

/// **A refused stage yields an EMPTY set, and a stage bounded by it is bounded to NOTHING.**
///
/// Empty is bounded-to-nothing; absent is unbounded. Collapsing them turns a failed stage into a
/// global search — a different question, answered confidently, with a full page of plausible
/// results and nothing to distinguish it from a real answer.
///
/// It needs no new mechanism, and that is the point of asserting it here rather than inventing one:
/// `ARRAY(SELECT id FROM <empty cte>)` is `'{}'`, which the fragments already read as zero rows,
/// while `NULL` is what they read as unbounded.
#[test]
fn a_stage_downstream_of_a_refusal_is_bounded_to_nothing_rather_than_unbounded() {
    let v = build(
        vec![
            find_stage("wide", ActName::FindAboutAnywhere, None),
            find_stage(
                "narrowed",
                ActName::FindExact,
                Some(StageInput::Upstream {
                    relation: StageRelation::Bound,
                    stage: StageName::parse("wide").unwrap(),
                }),
            ),
        ],
        vec!["narrowed"],
    );
    let c = compile(&v, test_profile()).expect("compiles");

    let refused = c
        .sql
        .split(r#""wide" AS ("#)
        .nth(1)
        .expect("wide CTE")
        .to_string();
    let refused = refused.split("\n)").next().unwrap();
    assert!(
        refused.contains("WHERE false"),
        "a refused stage produces nothing: {refused}"
    );

    let downstream = c
        .sql
        .split(r#""narrowed" AS ("#)
        .nth(1)
        .expect("narrowed CTE")
        .to_string();
    let downstream = downstream.split("\n)").next().unwrap();
    assert!(
        downstream.contains(r#"ARRAY(SELECT id FROM "wide")"#),
        "the downstream stage still takes its bound from the refused stage — an EMPTY array, \
         never a NULL that would read as unbounded: {downstream}"
    );
    assert!(
        !downstream.contains("NULL::uuid[]"),
        "a refusal must not silently widen the stage below it to the whole corpus: {downstream}"
    );
}

/// The other half of the pair: an absent QUESTION is still the caller's, and still refuses.
///
/// Written because the change above moves one refusal and it would be easy to move both. These two
/// absences are now distinct in the type, and this is what keeps them distinct in behaviour — a
/// composition with no intention has no words to search for, which no amount of server-side
/// embedding can supply.
#[test]
fn a_stage_with_no_threaded_question_still_refuses_as_the_callers_omission() {
    // Built without `validate` on purpose: the validator refuses this first, so routing through it
    // would test the validator and leave the compiler's own arm unwitnessed. `compile` is public
    // and does not require its caller to have validated on the same tick.
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: StageName::parse("quoted").unwrap(),
                with: vec![],
            }],
        },
        // The find stage carries no intention of its own — which is what `MissingIntention` is
        // about now that the field is per stage. `[2026-08-12]`
        stages: vec![find_stage_without_intention(
            "quoted",
            ActName::FindExact,
            None,
        )],
    };
    let refusals = validate(&c).expect_err("no intention is refused statically");
    assert!(refusals
        .iter()
        .any(|r| r.reason == RefusalReason::MissingIntention));
    assert!(
        !refusals
            .iter()
            .any(|r| r.reason == RefusalReason::EmbeddingUnavailable),
        "a missing question is decided before anything embeds; got: {refusals:?}"
    );
}

/// `survey` is refused BEFORE the compiler ever sees it.
///
/// **`[re-pointed — 2026-08-12]`** This asserted that both acts compile to the deliberately-absent
/// placeholder — a statement that cannot silently return wrong rows, but also one that fails at
/// EXECUTION, which is invisible until a door ships. Their fragments take arguments no slot supplies
/// (`p_depth`/`p_gamma`, `p_lens`), so they left `CALLABLE_FRAGMENTS` and now refuse statically.
///
/// `[narrowed to survey — 2026-08-14]` **`follow-from` rejoined the map.** Its `p_depth`/`p_gamma`
/// turned out to want CONSTANTS rather than slots — both are definitional, fixed by the act rather
/// than chosen by a caller — so the compiler writes them, and the only thing the act needed a slot
/// for was a bound, which `inputs: Vec<StageInput>` now carries. The positive half is asserted
/// beside this, in `a_walk_compiles_to_the_provenance_core_and_carries_its_via_column`: without it,
/// this test could be kept green by deleting an act from a list.
///
/// Re-pointed rather than deleted, for the reason this file has re-pointed several times before
/// (`[2026-08-08]`, `[2026-08-09]` twice, `[2026-08-10]`): the property still needs a witness, and
/// it is now the honest one — an act that cannot answer refuses instead of compiling to a function
/// that does not exist.
///
/// **What it asserts is narrower than "the compiler never sees this", exactly as
/// `a_multi_id_anchor_bound_is_refused_before_the_compiler_ever_sees_it` is.** It calls `validate`
/// and never `compile`; that the placeholder arm in `emit_act_body` is therefore unreachable is an
/// INFERENCE from `ValidatedComposition` being parse-don't-validate, not something this test
/// carries. The arm survives as an unreachable drift guard, which is why it is still emitted for a
/// hypothetical eighth act that declared itself into the family without a fragment.
#[test]
fn the_unmodelled_acts_are_refused_before_the_compiler_ever_sees_them() {
    let act = ActName::Survey;
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: StageName::parse("s").unwrap(),
                with: vec![],
            }],
        },
        stages: vec![find_stage("s", act.clone(), None)],
    };
    let errs = validate(&c).expect_err("the act has no fragment this surface can emit");
    assert!(
        errs.iter()
            .any(|e| e.reason == RefusalReason::NotSeparablyReachable),
        "{act:?} must refuse as unreachable rather than compile to `__temper_unbound_act`; \
         got {errs:?}"
    );
}

/// **The positive half**: a walk compiles to the provenance core, carries its `via` column, and
/// routes its two sets to the two slots the relation names.
///
/// `[added — 2026-08-14]` Without this, `the_unmodelled_acts_are_refused_before_the_compiler_ever_
/// sees_them` could be kept green by deleting an act from a list, and nothing would say whether the
/// act had become reachable or merely stopped being checked.
///
/// **The seed/bound assertion is the load-bearing one.** Both are `uuid[]`, they sit adjacent in the
/// signature, and routing a seed into `p_bound_ids` compiles a walk that can only return what was
/// already in its own seed set — a stage that looks like it worked and can never reach a neighbour.
/// Asserting only that both appear would pass against exactly that bug, so the assertion is about
/// ORDER: the seed expression must precede the bound expression in the emitted call.
#[test]
fn a_walk_compiles_to_the_provenance_core_and_carries_its_via_column() {
    let seeds = vec![Uuid::now_v7()];
    let bound = vec![Uuid::now_v7(), Uuid::now_v7()];
    let walk = StageNode::Act(ActInvocation {
        name: StageName::parse("near").unwrap(),
        act: ActName::FollowFrom,
        // A walk asks the corpus no question — it walks from a set it is handed.
        intention: None,
        inputs: vec![
            StageInput::Caller {
                relation: StageRelation::Seed,
                ids: IdSet {
                    kind: IdKind::Resource,
                    provenance: None,
                    ids: seeds.clone(),
                },
            },
            StageInput::Caller {
                relation: StageRelation::Bound,
                ids: IdSet {
                    kind: IdKind::Resource,
                    provenance: None,
                    ids: bound.clone(),
                },
            },
        ],
        terms: Default::default(),
        resource_filter: None,
        edge_filter: Some(EdgeFilter {
            edge_kinds: vec![EdgeKind::LeadsTo],
            labels: vec!["cites".to_string()],
        }),
        properties: vec![],
    });
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: StageName::parse("near").unwrap(),
                with: vec![],
            }],
        },
        stages: vec![walk],
    };
    let v = validate(&c).expect("a seeded, bounded walk with an edge filter is well-formed");
    let compiled = compile(&v, ProfileId::new()).expect("it compiles");
    let sql = &compiled.sql;

    assert!(
        sql.contains("__temper_ungated_follow_from"),
        "the walk emits the ungated core, not the absent placeholder; got:\n{sql}"
    );
    assert!(
        !sql.contains("__temper_unbound_act"),
        "the placeholder is what this act used to compile to; got:\n{sql}"
    );

    // `via` rides the stage contract as a fourth column, on the walk's arm and on the final select.
    assert!(
        sql.contains("graph_score::double precision AS quantity, via"),
        "the walk projects `via` beside its quantity; got:\n{sql}"
    );
    assert!(
        sql.contains("id, kind, quantity, via,"),
        "the final select carries `via` in its shared column list; got:\n{sql}"
    );

    // The two definitional constants the compiler writes rather than binds.
    assert!(
        sql.contains(", 2, 0.5::double precision,"),
        "depth and gamma are constants fixed by the act, not caller-bound parameters; got:\n{sql}"
    );

    // **POSITION, not presence.** The two id sets are both `uuid[]`, sit in the same call, and mean
    // opposite things; asserting that each merely appears would pass against a body that swapped
    // them. The fragment's signature is positional, so the assertion is too — argument 1 is the
    // seed set and argument 6 is the bound, per `20260814000030`.
    let call = core_call_args(sql, "__temper_ungated_follow_from");
    let args: Vec<&str> = split_top_level(&call);
    assert_eq!(
        args.len(),
        8,
        "the walk core takes eight arguments; got:\n{call}"
    );

    let seed_bind = args[1];
    let bound_bind = args[6];
    assert!(
        seed_bind.ends_with("::uuid[]") && seed_bind.starts_with('$'),
        "argument 1 is the SEED set; got `{seed_bind}` in:\n{call}"
    );
    assert!(
        bound_bind.ends_with("::uuid[]") && bound_bind.starts_with('$'),
        "argument 6 is the BOUND set; got `{bound_bind}` in:\n{call}"
    );
    assert_ne!(
        seed_bind, bound_bind,
        "two slots, two binds — one bind in both would be a walk bounded to its own seeds, which \
         is a different act. got:\n{call}"
    );
    // The seed is bound before the bound is, because `narrowing_for` walks `inputs` in order and
    // this plan declares the seed first — which is what makes the two binds distinguishable at all.
    assert!(
        seed_bind < bound_bind,
        "swapping these compiles a walk that can only return what was already in its own seed \
         set — a stage that looks like it worked and can never reach a neighbour. got:\n{call}"
    );

    // Both edge axes reach the fragment — the label one has never existed in the incumbent walk.
    assert!(
        args[4].ends_with("::text[]") && args[5].ends_with("::text[]"),
        "arguments 4 and 5 are the kind and label axes; got:\n{call}"
    );
}

/// A walk with no edge filter passes NULL on both axes — never `'{{}}'`, which the fragment reads as
/// a different question.
#[test]
fn a_walk_without_an_edge_filter_narrows_on_neither_axis() {
    let walk = StageNode::Act(ActInvocation {
        name: StageName::parse("near").unwrap(),
        act: ActName::FollowFrom,
        intention: None,
        inputs: vec![StageInput::Caller {
            relation: StageRelation::Seed,
            ids: IdSet {
                kind: IdKind::Resource,
                provenance: None,
                ids: vec![Uuid::now_v7()],
            },
        }],
        terms: Default::default(),
        resource_filter: None,
        edge_filter: None,
        properties: vec![],
    });
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: StageName::parse("near").unwrap(),
                with: vec![],
            }],
        },
        stages: vec![walk],
    };
    let v = validate(&c).expect("an unfiltered walk is well-formed");
    let compiled = compile(&v, ProfileId::new()).expect("it compiles");
    let call = core_call_args(&compiled.sql, "__temper_ungated_follow_from");
    assert!(
        call.contains("NULL::text[], NULL::text[]"),
        "both edge axes are NULL — unbounded — and never an empty array; got:\n{call}"
    );
    // And the bound slot too: no bound input means no narrowing, not narrowing to nothing.
    assert!(
        call.contains("NULL::uuid[]"),
        "an absent bound is NULL, never '{{}}'; got:\n{call}"
    );
}

/// The argument list of a core call, with balanced parentheses honoured.
///
/// A naive split on `)` truncates at `(SELECT ids FROM __temper_vis)` — the FIRST argument — so
/// every assertion downstream would read one argument and pass or fail for the wrong reason.
fn core_call_args(sql: &str, core: &str) -> String {
    let open = format!("{core}(");
    let start = sql.find(&open).expect("the core is called") + open.len();
    let mut depth = 1usize;
    for (i, c) in sql[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return sql[start..start + i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced parentheses in the emitted call");
}

/// Split an argument list on commas that are NOT inside nested parentheses.
fn split_top_level(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut last = 0usize;
    for (i, c) in args.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(args[last..i].trim());
                last = i + 1;
            }
            _ => {}
        }
    }
    out.push(args[last..].trim());
    out
}

// ─── Post-review fixes ──────────────────────────────────────────────────────────────────────────

fn caller_set(kind: IdKind, ids: Vec<Uuid>) -> StageInput {
    StageInput::Caller {
        relation: StageRelation::Bound,
        ids: IdSet {
            kind,
            provenance: None,
            ids,
        },
    }
}

/// A Context/Cogmap bound goes to the ANCHOR PAIR, never into `p_bound_ids uuid[]`.
///
/// Routing it to the array is a **confident empty**: `find-exact` declares
/// `accepts_bounds: [Resource, Context, Cogmap]`, so the plan validates, and then cogmap uuids get
/// compared against `r.id` — zero rows, 200 OK, and nothing saying the narrowing was nonsense.
#[test]
fn a_cogmap_bound_is_emitted_as_the_anchor_pair_not_as_a_resource_id_array() {
    let cogmap = Uuid::now_v7();
    let v = build(
        vec![find_stage(
            "hits",
            ActName::FindExact,
            Some(caller_set(IdKind::Cogmap, vec![cogmap])),
        )],
        vec!["hits"],
    );
    let c = compile(&v, test_profile()).expect("compiles");

    assert!(
        c.sql.contains("'kb_cogmaps'::varchar"),
        "a cogmap bound must reach the anchor-table slot; got:\n{}",
        c.sql
    );
    // The array slot must be explicitly unbounded — NOT the cogmap id, and NOT '{}' (which would
    // mean bounded-to-nothing and return zero rows for a different, equally silent reason).
    assert!(
        c.sql.contains("NULL::uuid[]"),
        "the resource-id array must be unbounded when the narrowing is an anchor; got:\n{}",
        c.sql
    );
    assert!(
        !c.sql.contains(&cogmap.to_string()),
        "the anchor id must be bound, never interpolated"
    );
}

/// An anchor slot holds ONE id; an `IdSet` holds N. Two is a refusal, never a silent first-wins.
///
/// Spec §9 names this cardinality gap. Anchoring on `ids[0]` would answer a different question than
/// the one asked while looking like a successful narrowing — the failure mode this whole arc is
/// against.
///
/// **`[moved to the validator — 2026-08-09, Pete]`** It used to build the plan, compile it, and read an `Err` —
/// and that `Err` aborted the whole composition, so a healthy stage beside the bad one was refused
/// for its neighbour's mistake. The cardinality is static, so it is refused at validation now and
/// arrives in the 400 with every other refusal.
///
/// Re-pointed rather than deleted, for the reason this file has re-pointed twice before: the
/// property still needs a witness, and deleting a test when its expected value changes retires the
/// only thing holding the distinction.
///
/// **What it asserts is narrower than "the compiler never sees this", and the difference matters.**
/// It calls `validate` and never `compile`; that the compiler is therefore unreachable is an
/// INFERENCE from `ValidatedComposition` being parse-don't-validate, not something this test
/// carries. No test can carry it — constructing a `ValidatedComposition` that skipped the check is
/// impossible by design, which is also why the compiler's arm survives as an unreachable drift
/// guard (same shape as `bound_expr`'s seed arm). An earlier version of this comment claimed the
/// assertion outright; review caught the overclaim.
#[test]
fn a_multi_id_anchor_bound_is_refused_before_the_compiler_ever_sees_it() {
    let c = Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: StageName::parse("hits").unwrap(),
                with: vec![],
            }],
        },
        stages: vec![find_stage(
            "hits",
            ActName::FindExact,
            Some(caller_set(
                IdKind::Context,
                vec![Uuid::now_v7(), Uuid::now_v7()],
            )),
        )],
    };

    let errs = validate(&c).expect_err("two anchor ids must refuse");
    assert!(
        errs.iter()
            .any(|e| e.reason == RefusalReason::AnchorTakesOneId
                && e.stage.as_ref().is_some_and(|s| s.as_str() == "hits")),
        "got: {errs:?}"
    );
    assert!(
        !errs
            .iter()
            .any(|e| e.reason == RefusalReason::UnsupportedBoundKind),
        "the KIND is accepted — saying otherwise teaches the caller to stop sending context \
         bounds, when the fix is to send one: {errs:?}"
    );
}

/// **A term above its published ceiling is CLAMPED, and the clamped value is what runs.**
///
/// The contract: *"A term the act accepts, above its published ceiling → CLAMPED. The value used
/// comes back in `StageResult.terms_applied`."* Binding the caller's number instead makes the
/// ceiling a decoration — `find-exact` publishes `limit: 50` and would happily draw 500 — and then
/// `terms_applied` has to either report a value that did not run or become a second opinion about
/// what did. One function decides it, and both the bind and the report read that function.
#[test]
fn a_term_above_its_published_ceiling_is_clamped_to_the_ceiling_that_was_published() {
    let mut node = find_stage("hits", ActName::FindExact, None);
    if let StageNode::Act(a) = &mut node {
        a.terms.insert(BoundTerm::Limit, 500);
    }
    let v = build(vec![node], vec!["hits"]);
    let c = compile(&v, test_profile()).expect("compiles");

    let ints: Vec<i64> = c
        .binds
        .iter()
        .filter_map(|b| match b {
            QueryBind::Int(i) => Some(*i),
            _ => None,
        })
        .collect();
    assert!(
        ints.contains(&50),
        "the published ceiling is what runs; got binds {ints:?}"
    );
    assert!(
        !ints.contains(&500),
        "the caller's number must not reach the fragment: {ints:?}"
    );
}

/// Below the ceiling, the caller's own value is used — so the rule above is a CEILING and not a
/// fixed page size.
#[test]
fn a_term_below_its_ceiling_is_used_as_the_caller_asked() {
    let mut node = find_stage("hits", ActName::FindExact, None);
    if let StageNode::Act(a) = &mut node {
        a.terms.insert(BoundTerm::Limit, 7);
    }
    let v = build(vec![node], vec!["hits"]);
    let c = compile(&v, test_profile()).expect("compiles");
    assert!(c.binds.iter().any(|b| matches!(b, QueryBind::Int(7))));
}

/// Paging terms are BOUND, not dropped — whether the caller declared them or not.
///
/// Emitting literal `NULL`/`0` means a plan declaring `limit: 10` compiles to the entire match set —
/// the wide-then-hydrate cost `20260806000020` measured at 1,883 rows for a request asking for ten,
/// and it leaves the declared `bound_ceilings` unenforced. Worse in a chain, where an unlimited
/// upstream feeds every id into a bounded downstream stage.
///
/// `[re-pointed — 2026-08-10, ADJ-11]` The second half of this test asserted the opposite of what it
/// now asserts, and the old expectation was not wrong so much as *superseded*: an undeclared limit
/// used to emit literal `NULL`, which Postgres reads as unbounded. That is the SAME 1,883-row
/// failure the first half exists to prevent, reached by omitting the term instead of by dropping it
/// — and it was reachable from the request shape agents send most, the one that names no page size.
/// `applied_terms` now defaults an omitted `limit` to the act's published ceiling, so the slot
/// carries a bind here too. `NULL`-means-unbounded is still the rule for the **id array** (see
/// `a_stage_downstream_of_a_refusal_is_bounded_to_nothing_rather_than_unbounded`); it is no longer
/// the rule for the limit.
///
/// The test name moved with the property. It said `declared_limit_and_offset_…`, which was only ever
/// true of the first half and is now false about the second.
#[test]
fn paging_terms_reach_the_fragment_as_binds_whether_declared_or_defaulted() {
    let mut node = find_stage("hits", ActName::FindExact, None);
    if let StageNode::Act(inv) = &mut node {
        inv.terms.insert(BoundTerm::Limit, 10);
        inv.terms.insert(BoundTerm::Offset, 20);
    }
    let v = build(vec![node], vec!["hits"]);
    let c = compile(&v, test_profile()).expect("compiles");

    let ints: Vec<i64> = c
        .binds
        .iter()
        .filter_map(|b| match b {
            QueryBind::Int(i) => Some(*i),
            _ => None,
        })
        .collect();
    assert!(
        ints.contains(&10) && ints.contains(&20),
        "the declared limit and offset must be bound; got {:?}",
        c.binds
    );

    // And a stage that declares NEITHER gets the act's published ceiling in the limit slot, bound
    // the same way — not a literal `NULL` that would return the whole visible match set to a caller
    // who simply did not say how many they wanted.
    let bare = build(
        vec![find_stage("plain", ActName::FindExact, None)],
        vec!["plain"],
    );
    let cb = compile(&bare, test_profile()).expect("compiles");
    let bare_ints: Vec<i64> = cb
        .binds
        .iter()
        .filter_map(|b| match b {
            QueryBind::Int(i) => Some(*i),
            _ => None,
        })
        .collect();
    // Read the ceiling off the declaration rather than restating `50`. A second copy of the number
    // would keep this test green through a ceiling change that moved the emitted SQL underneath it.
    let ceiling = *declaration(&ActName::FindExact)
        .expect("find-exact is declared")
        .bound_ceilings
        .get(&BoundTerm::Limit)
        .expect("find-exact publishes a limit ceiling");
    assert!(
        bare_ints.contains(&ceiling),
        "an undeclared limit must bind the published ceiling {ceiling}; got {:?}",
        cb.binds
    );
    assert!(
        !cb.sql.contains("NULL, 0)"),
        "an undeclared limit must not compile to unbounded; got:\n{}",
        cb.sql
    );
    // Offset is untouched by the default rule — it has no published ceiling to default to, and page
    // 1 is the right answer to a caller who named no page. It stays the literal `0`.
    assert!(
        cb.sql.contains(", 0)"),
        "an undeclared offset is still the literal 0; got:\n{}",
        cb.sql
    );
}

// ─── Task 8: one emitter, so there is no wrong set to pass ───────────────────────────────────────

/// The argument list of every ungated-core CALL in a compiled statement, `(` first.
///
/// The name also appears in each act's `-- act: … -> …` comment, and a naive scan counts those too —
/// then finds the `(` belonging to the call on the NEXT line and reports a duplicate that passes
/// every assertion. So a call is required to have its `(` adjacent to the name: no whitespace
/// between them.
fn ungated_core_calls(sql: &str) -> Vec<String> {
    sql.match_indices("__temper_ungated_")
        .filter_map(|(i, _)| {
            let rest = &sql[i..];
            let open = rest.find('(')?;
            rest[..open]
                .chars()
                .all(|c| !c.is_whitespace())
                .then(|| rest[open..].to_string())
        })
        .collect()
}

/// **Every ungated-core call takes its ids from the hoisted relation and from nothing else.**
///
/// This is the failure the CI tripwire cannot see. That script pins *where* a core is called; the
/// realistic bug is not a rogue call site but an approved one passing `stage_2` where it should pass
/// the visible set — CI green, RBAC bypassed, and every row still looking plausible. Closed
/// structurally instead: one emitter, with the id source not a parameter of it.
///
/// Asserted over a composition where a find stage IS narrowed by an upstream stage, because that is
/// the shape in which the two arrays are both in scope and confusable. The upstream ids must appear
/// as the BOUND and never as the visible set.
#[test]
fn every_ungated_core_call_takes_its_ids_from_the_hoisted_relation_and_nothing_else() {
    let v = build(
        vec![
            find_stage("hits", ActName::FindExact, None),
            find_stage_embedded(
                "narrowed",
                ActName::FindAboutWithin,
                Some(StageInput::Upstream {
                    relation: StageRelation::Bound,
                    stage: StageName::parse("hits").unwrap(),
                }),
            ),
            // `[added — 2026-08-14, found in adversarial review]` **The `Selection` arm was absent
            // from the guard that carries this property's name.** `emit_ungated_core_call` became a
            // two-variant enum when `find-resources-with` landed, and this test kept building only
            // `Find` stages — so the invariant was verified for the shape that already existed and
            // not for the one being added. Its first argument was checked only incidentally, inside
            // a different test's expected-string.
            selection("sel"),
        ],
        vec!["narrowed"],
    );
    let c = compile(&v, test_profile()).expect("compiles");

    let calls = ungated_core_calls(&c.sql);
    assert_eq!(
        calls.len(),
        3,
        "both find stages AND the selection must reach an ungated core; got {calls:?} in:\n{}",
        c.sql
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("__temper_ungated_find_resources_with")),
        "the selection variant must be among the calls this guard inspects, or it verifies only \
         the arm that already existed: {calls:?}"
    );

    // The literal is written out rather than imported so that changing the emitter's id source has
    // to change this test too — an assertion that reads the same constant the code emits would
    // agree with any value, including the wrong one.
    let expected = "((SELECT ids FROM __temper_vis),";
    for call in &calls {
        assert!(
            call.starts_with(expected),
            "an ungated core was handed something other than the hoisted visible set as its first \
             argument. Expected to start `{expected}`, got `{}`",
            &call[..call.len().min(90)]
        );
    }

    // The upstream ids are in scope in the same statement, which is exactly why the assertion above
    // has teeth: they reach the call as the BOUND, in a different slot.
    assert!(
        c.sql.contains(r#"ARRAY(SELECT id FROM "hits")"#),
        "the upstream stage's ids must still narrow the downstream stage; got:\n{}",
        c.sql
    );
    assert!(
        !c.sql
            .contains(r#"__temper_ungated_find_wide(ARRAY(SELECT id FROM "hits")"#),
        "the upstream set must never land in the visible-set slot"
    );
}

/// The hoisted relation cannot be shadowed by a stage, and that is a property of the NAME.
///
/// `__temper_vis` now carries the RBAC verdict for every stage in the statement, so a caller-chosen
/// stage name colliding with it would be an authorization question decided by a naming accident.
/// `StageName::parse` requires the first character to be an ASCII lowercase letter, so no stage can
/// ever be called `__temper_vis` — the collision is impossible by construction rather than rejected
/// by a check that someone has to remember to write.
#[test]
fn no_stage_can_be_named_after_the_hoisted_visibility_relation() {
    assert!(
        StageName::parse("__temper_vis").is_none(),
        "a stage able to take the hoisted relation's name could shadow the verdict every core call \
         reads"
    );
    // And the compiler really does use that unreachable name, so the guarantee above is about the
    // identifier actually emitted rather than about a name nothing uses.
    let c = compile(&plan_one_stage(), test_profile()).expect("compiles");
    assert!(
        c.sql.contains("__temper_vis AS MATERIALIZED"),
        "got:\n{}",
        c.sql
    );
}

/// A `find-resources-with` selection stage carrying every narrowing the act admits.
fn selection(name: &str) -> StageNode {
    StageNode::Act(ActInvocation {
        name: StageName::parse(name).unwrap(),
        act: ActName::FindResourcesWith,
        // **No intention, and no builder threads one onto it.** This is the one act that asks the
        // corpus nothing; the shape pass's requirement names three acts and this is not among them,
        // so an omitted intention here is well-formed rather than tolerated.
        intention: None,
        inputs: vec![],
        terms: Default::default(),
        resource_filter: Some(temper_core::types::query::ResourceFilter {
            doc_type: vec!["task".to_string(), "session".to_string()],
            tags: vec!["ci".to_string()],
            facets: vec![temper_core::types::query::FacetPredicate {
                key: "domain".to_string(),
                value: "search".to_string(),
            }],
            stage: Some("in-progress".to_string()),
            status: None,
            owner: Some("j-cole-taylor".to_string()),
            title_contains: Some("door".to_string()),
        }),
        edge_filter: None,
        properties: vec![],
    })
}

#[test]
fn the_selection_narrowings_bind_in_signature_order() {
    // The one real hazard in `selection_narrowings_for` is that its arguments are POSITIONAL: eight
    // slots, rendered by index, with no name at the call site to catch a transposition. The type
    // system catches some of it — `text[]` into a `text` slot does not typecheck — but `stage`,
    // `status`, `owner_handle` and `title_contains` are all `text`, so four of the eight are
    // mutually swappable without a compile error and without a runtime error. Only a rendering
    // assertion sees that.
    //
    // Asserting on the emitted SQL rather than on the bind vector alone is what makes it an
    // ORDER assertion: the binds carry the values, the text carries which slot each landed in.
    // A selection cannot be RETURNED — it orders nothing, so its rows have no quantity to score
    // them — so the plan pipes it into a find act, which is the shape a caller would really write
    // and the one the acceptance criterion names.
    let v = build(
        vec![selection("sel"), find_from("hits", "sel")],
        vec!["hits"],
    );
    let q = compile(&v, test_profile()).expect("selection compiles");

    let call = q
        .sql
        .lines()
        // The CALL line, not the `-- act:` comment line above it, which also names the core.
        .find(|l| l.contains("FROM __temper_ungated_find_resources_with("))
        .expect("the selection stage emits the selection core");

    // Signature order: doc_types, tags, facets, stage, status, owner_profile, owner_handle,
    // title_contains — then the anchor pair, then the principal LAST.
    let expected = "__temper_ungated_find_resources_with((SELECT ids FROM __temper_vis), \
                    $2::text[], $3::text[], $4::jsonb, $5::text, NULL::text, NULL::uuid, \
                    $6::text, $7::text, NULL, NULL, $1)";
    assert!(
        call.contains(expected),
        "narrowings must render in signature order.\n  expected: {expected}\n  got: {call}"
    );

    // `status` and `owner_profile` are the two the plan left unset, and they render as TYPED nulls
    // rather than bare ones: a bare NULL is ambiguous against a DEFAULTed parameter list.
    assert!(call.contains("NULL::text, NULL::uuid"), "got: {call}");

    // `owner` is ONE wire field and TWO slots. A handle takes the text slot and leaves the uuid one
    // null; a UUID string would do the reverse. Nothing parses `@me` here — the principal is a
    // bind, so a stage cannot be compiled to one profile's id and executed as another's.
    assert!(
        matches!(&q.binds[5], QueryBind::Text(t) if t == "j-cole-taylor"),
        "the handle spelling fills the handle slot: {:?}",
        q.binds
    );

    // And the whole point of the act: no quantity to rank on.
    assert!(
        q.sql.contains("NULL::double precision AS quantity"),
        "a selection ranks nothing, so its stage-contract quantity is NULL: {}",
        q.sql
    );
}
