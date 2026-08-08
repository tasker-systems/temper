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

fn plan_three_stages() -> ValidatedComposition {
    build(
        vec![
            ff_root("a", vec![Uuid::now_v7()]),
            ff_from("b", "a"),
            ff_from("c", "b"),
        ],
        vec!["c"],
    )
}

#[test]
fn every_caller_value_is_bound_and_none_is_interpolated() {
    // The security property, tested where it can be tested exhaustively: no database, no fixtures,
    // just the emitted text. A uuid appearing literally in the SQL is the failure.
    let (v, ids) = plan_with_caller_ids();
    let c = compile(&v, test_profile());
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
    let c = compile(&v, test_profile());
    assert!(c.sql.contains("hits AS ("));
    assert!(c.sql.contains("near AS ("));
    assert_eq!(c.cte_names.len(), 2);
}

#[test]
fn the_visibility_relation_is_materialized_once_no_matter_how_many_stages() {
    // Decision 019fcd13: one query time, one visibility computation. A per-stage recomputation is
    // the thing the single statement exists to collapse.
    let one = compile(&plan_one_stage(), test_profile());
    let three = compile(&plan_three_stages(), test_profile());
    assert_eq!(one.sql.matches("vis AS MATERIALIZED").count(), 1);
    assert_eq!(three.sql.matches("vis AS MATERIALIZED").count(), 1);
}

#[test]
fn a_downstream_stage_selects_ids_only_and_never_a_quantity() {
    // THE rule that keeps no-cross-act-ranking structural (spec §4). If a quantity can cross a
    // stage boundary, cross-act arithmetic becomes mechanically easy and nothing prevents it.
    let v = plan_two_stages("hits", "near");
    let c = compile(&v, test_profile());
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
    let c = compile(&v, test_profile());
    let hits_at = c.sql.find("hits AS (").unwrap();
    let near_at = c.sql.find("near AS (").unwrap();
    assert!(hits_at < near_at);
}
