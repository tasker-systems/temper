//! Pure emission tests for the query compiler — NO database. The security property (every caller
//! value is bound, never interpolated) and the structural properties (one visibility relation, ids
//! only across a stage boundary, dependency order) are all decidable from the emitted text alone.

use temper_core::types::ids::ProfileId;
use temper_core::types::query::{
    validate, ActInvocation, ActName, BoundsMode, Composition, IdKind, IdSet, OutcomeDeclaration,
    RefusalDisposition, ReturnSpec, StageInput, StageName, StageNode, ValidatedComposition,
};
use temper_substrate::readback::query_plan::{compile, QueryBind};
use uuid::Uuid;

fn test_profile() -> ProfileId {
    ProfileId::new()
}

/// A `follow-from` root fed a caller resource set — reachable, and valid at the end of beat B.
fn ff_root(name: &str, ids: Vec<Uuid>) -> StageNode {
    StageNode::Act(ActInvocation {
        name: StageName::parse(name).unwrap(),
        act: ActName::FollowFrom,
        input: Some(StageInput::Caller {
            ids: IdSet {
                kind: IdKind::Resource,
                provenance: None,
                ids,
            },
        }),
        bounds_mode: Some(BoundsMode::Seed),
        terms: Default::default(),
        resource_filter: None,
        edge_filter: None,
        properties: vec![],
    })
}

/// A `follow-from` stage seeded on an upstream stage.
fn ff_from(name: &str, upstream: &str) -> StageNode {
    StageNode::Act(ActInvocation {
        name: StageName::parse(name).unwrap(),
        act: ActName::FollowFrom,
        input: Some(StageInput::Upstream {
            stage: StageName::parse(upstream).unwrap(),
        }),
        bounds_mode: Some(BoundsMode::Seed),
        terms: Default::default(),
        resource_filter: None,
        edge_filter: None,
        properties: vec![],
    })
}

fn build(stages: Vec<StageNode>, returns: Vec<&str>) -> ValidatedComposition {
    let c = Composition {
        outcome: OutcomeDeclaration {
            description: "compile test".to_string(),
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
        bounds: Default::default(),
        stages,
    };
    validate(&c).expect("plan is valid")
}

fn plan_with_caller_ids() -> (ValidatedComposition, Vec<Uuid>) {
    let ids = vec![Uuid::now_v7(), Uuid::now_v7()];
    (build(vec![ff_root("hits", ids.clone())], vec!["hits"]), ids)
}

fn plan_two_stages(root: &str, downstream: &str) -> ValidatedComposition {
    build(
        vec![
            ff_root(root, vec![Uuid::now_v7()]),
            ff_from(downstream, root),
        ],
        vec![downstream],
    )
}

fn plan_two_stages_declared_backwards(root: &str, downstream: &str) -> ValidatedComposition {
    // The downstream stage is DECLARED first; validate()'s topological order must still put the
    // root before it, and the compiler must emit in that order.
    build(
        vec![
            ff_from(downstream, root),
            ff_root(root, vec![Uuid::now_v7()]),
        ],
        vec![downstream],
    )
}

fn plan_one_stage() -> ValidatedComposition {
    build(vec![ff_root("hits", vec![Uuid::now_v7()])], vec!["hits"])
}

/// One `find-exact` stage — an act that reaches a real fragment, which the compute-once property
/// needs: a `follow-from` stage still emits the placeholder and consults no visibility relation at
/// all, so a plan built from those could satisfy the assertion while proving nothing.
fn plan_one_find() -> ValidatedComposition {
    build_with_intention(vec![find_stage("a", ActName::FindExact, None)], vec!["a"])
}

/// Three chained `find-exact` stages. `find-exact` throughout rather than a `find-about-*` mix so
/// the plan compiles with no embedding; the property under test is about the gate, not the arm.
fn plan_three_finds() -> ValidatedComposition {
    build_with_intention(
        vec![
            find_stage("a", ActName::FindExact, None),
            find_stage(
                "b",
                ActName::FindExact,
                Some(StageInput::Upstream {
                    stage: StageName::parse("a").unwrap(),
                }),
            ),
            find_stage(
                "c",
                ActName::FindExact,
                Some(StageInput::Upstream {
                    stage: StageName::parse("b").unwrap(),
                }),
            ),
        ],
        vec!["c"],
    )
}

// `plan_three_stages` (three chained `follow-from` stages) lived here and is deleted rather than
// `#[allow(dead_code)]`d. Its only consumer was the compute-once test, which now needs
// `plan_three_finds`: a `follow-from` stage emits the placeholder and consults no visibility
// relation at all, so a three-stage plan built from those would satisfy the assertion while proving
// nothing. Rust noticing it went unused is the check working.

#[test]
fn every_caller_value_is_bound_and_none_is_interpolated() {
    // The security property, tested where it can be tested exhaustively: no database, no fixtures,
    // just the emitted text. A uuid appearing literally in the SQL is the failure.
    let (v, ids) = plan_with_caller_ids();
    let c = compile(&v, test_profile(), None).expect("compiles");
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
    let c = compile(&v, test_profile(), None).expect("compiles");
    assert!(c.sql.contains("hits AS ("));
    assert!(c.sql.contains("near AS ("));
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
    // (`20260808000040`) do not call it, so any second occurrence means a stage went back to gating
    // for itself.
    let one = compile(&plan_one_find(), test_profile(), None).expect("compiles");
    let three = compile(&plan_three_finds(), test_profile(), None).expect("compiles");

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
    let c = compile(&v, test_profile(), None).expect("compiles");
    let downstream = c.sql.split("near AS (").nth(1).expect("near CTE present");
    assert!(
        downstream.contains("SELECT id FROM hits"),
        "got: {downstream}"
    );
    assert!(
        !downstream.contains("quantity FROM hits"),
        "a quantity crossed a stage boundary"
    );
}

#[test]
fn stages_are_emitted_in_dependency_order() {
    // The compiler consumes ValidatedComposition::ordered(); a CTE referencing one declared later
    // would not parse.
    let v = plan_two_stages_declared_backwards("hits", "near");
    let c = compile(&v, test_profile(), None).expect("compiles");
    let hits_at = c.sql.find("hits AS (").unwrap();
    let near_at = c.sql.find("near AS (").unwrap();
    assert!(hits_at < near_at);
}

// ─── Beat D: the find acts emit real fragments ──────────────────────────────────────────────────

use temper_core::types::query::{BoundTerm, Intention, RefusalReason};

fn find_stage(name: &str, act: ActName, input: Option<StageInput>) -> StageNode {
    StageNode::Act(ActInvocation {
        name: StageName::parse(name).unwrap(),
        act,
        input,
        bounds_mode: Some(BoundsMode::Bound),
        terms: Default::default(),
        resource_filter: None,
        edge_filter: None,
        properties: vec![],
    })
}

/// `build`, plus a threaded intention — which the find acts require.
fn build_with_intention(stages: Vec<StageNode>, returns: Vec<&str>) -> ValidatedComposition {
    let c = Composition {
        outcome: OutcomeDeclaration {
            description: "compile test".to_string(),
            returns: returns
                .into_iter()
                .map(|s| ReturnSpec {
                    stage: StageName::parse(s).unwrap(),
                    fields: vec![],
                })
                .collect(),
        },
        intention: Some(Intention {
            query: "salience".to_string(),
            embedded: true,
        }),
        on_stage_refusal: RefusalDisposition::Halt,
        meta_detail: Default::default(),
        bounds: Default::default(),
        stages,
    };
    validate(&c).expect("plan is valid")
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
    let v = build_with_intention(
        vec![
            ff_root("seeds", vec![Uuid::now_v7()]),
            find_stage(
                "narrowed",
                ActName::FindAboutWithin,
                Some(StageInput::Upstream {
                    stage: StageName::parse("seeds").unwrap(),
                }),
            ),
        ],
        vec!["narrowed"],
    );
    let emb = an_embedding();
    let c = compile(&v, test_profile(), Some(&emb)).expect("compiles");

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
        c.sql.contains("ARRAY(SELECT id FROM seeds)"),
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
    let v = build_with_intention(
        vec![find_stage("hits", ActName::FindExact, None)],
        vec!["hits"],
    );
    let c = compile(&v, test_profile(), None).expect("find-exact needs no embedding");

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

/// **A `find-about-*` stage with no embedding REFUSES.**
///
/// The delta-2 property: a refusal is its own state, distinct from failure and from honest-empty.
/// Fails against a builder that binds NULL for a missing embedding — which would run a vector
/// search on nothing and report an empty result, collapsing "I chose not to embed" and "I cannot
/// embed" into one indistinguishable answer.
#[test]
fn a_wide_find_without_an_embedding_refuses_rather_than_binding_null() {
    let v = build_with_intention(
        vec![find_stage("wide", ActName::FindAboutAnywhere, None)],
        vec!["wide"],
    );
    let err = compile(&v, test_profile(), None)
        .expect_err("no embedding must refuse, not compile to a NULL bind");

    assert_eq!(err.reason, RefusalReason::MissingIntention);
    assert_eq!(err.stage.as_ref().map(|s| s.as_str()), Some("wide"));

    // And the same plan WITH an embedding compiles — so the refusal is about the embedding and not
    // about the plan being malformed in some other way.
    let emb = an_embedding();
    assert!(compile(&v, test_profile(), Some(&emb)).is_ok());
}

/// `follow-from` and `survey` keep the deliberately-absent placeholder.
///
/// Their fragments take arguments no slot supplies (`p_depth`/`p_gamma`, `p_lens`), so emitting a
/// real call would mean inventing a value. A function that does not exist fails loudly instead.
#[test]
fn the_unmodelled_acts_still_emit_the_absent_placeholder() {
    let c = compile(&plan_one_stage(), test_profile(), None).expect("compiles");
    assert!(
        c.sql.contains("__temper_unbound_act("),
        "follow-from has no modelled fragment and must keep the loud placeholder; got:\n{}",
        c.sql
    );
    assert!(
        !c.sql.contains("query_find_"),
        "and must not be given a find twin, which is not its mechanic"
    );
}

// ─── Post-review fixes ──────────────────────────────────────────────────────────────────────────

fn caller_set(kind: IdKind, ids: Vec<Uuid>) -> StageInput {
    StageInput::Caller {
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
    let v = build_with_intention(
        vec![find_stage(
            "hits",
            ActName::FindExact,
            Some(caller_set(IdKind::Cogmap, vec![cogmap])),
        )],
        vec!["hits"],
    );
    let c = compile(&v, test_profile(), None).expect("compiles");

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
#[test]
fn a_multi_id_anchor_bound_refuses_rather_than_anchoring_on_the_first() {
    let v = build_with_intention(
        vec![find_stage(
            "hits",
            ActName::FindExact,
            Some(caller_set(
                IdKind::Context,
                vec![Uuid::now_v7(), Uuid::now_v7()],
            )),
        )],
        vec!["hits"],
    );
    let err = compile(&v, test_profile(), None).expect_err("two anchor ids must refuse");
    assert_eq!(err.reason, RefusalReason::UnsupportedBoundKind);
    assert_eq!(err.stage.as_ref().map(|s| s.as_str()), Some("hits"));
}

/// Declared paging terms are BOUND, not dropped.
///
/// Emitting literal `NULL`/`0` means a plan declaring `limit: 10` compiles to the entire match set —
/// the wide-then-hydrate cost `20260806000020` measured at 1,883 rows for a request asking for ten,
/// and it leaves the declared `bound_ceilings` unenforced. Worse in a chain, where an unlimited
/// upstream feeds every id into a bounded downstream stage.
#[test]
fn declared_limit_and_offset_reach_the_fragment_as_binds() {
    let mut node = find_stage("hits", ActName::FindExact, None);
    if let StageNode::Act(inv) = &mut node {
        inv.terms.insert(BoundTerm::Limit, 10);
        inv.terms.insert(BoundTerm::Offset, 20);
    }
    let v = build_with_intention(vec![node], vec!["hits"]);
    let c = compile(&v, test_profile(), None).expect("compiles");

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
    // And a stage that declares neither still says "unbounded" rather than "zero rows".
    let bare = build_with_intention(
        vec![find_stage("plain", ActName::FindExact, None)],
        vec!["plain"],
    );
    let cb = compile(&bare, test_profile(), None).expect("compiles");
    assert!(
        cb.sql.contains("NULL, 0)"),
        "an undeclared limit is NULL (unbounded) and offset is 0; got:\n{}",
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
    let v = build_with_intention(
        vec![
            find_stage("hits", ActName::FindExact, None),
            find_stage(
                "narrowed",
                ActName::FindAboutWithin,
                Some(StageInput::Upstream {
                    stage: StageName::parse("hits").unwrap(),
                }),
            ),
        ],
        vec!["narrowed"],
    );
    let emb = an_embedding();
    let c = compile(&v, test_profile(), Some(&emb)).expect("compiles");

    let calls = ungated_core_calls(&c.sql);
    assert_eq!(
        calls.len(),
        2,
        "both find stages must reach an ungated core; got {calls:?} in:\n{}",
        c.sql
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
        c.sql.contains("ARRAY(SELECT id FROM hits)"),
        "the upstream stage's ids must still narrow the downstream stage; got:\n{}",
        c.sql
    );
    assert!(
        !c.sql
            .contains("__temper_ungated_find_wide(ARRAY(SELECT id FROM hits)"),
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
    let c = compile(&plan_one_stage(), test_profile(), None).expect("compiles");
    assert!(
        c.sql.contains("__temper_vis AS MATERIALIZED"),
        "got:\n{}",
        c.sql
    );
}
