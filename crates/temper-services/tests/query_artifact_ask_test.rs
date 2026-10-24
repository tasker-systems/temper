#![cfg(feature = "test-db")]

//! **A composition asking data artifacts, through the whole `/api/query` read path** — the shapes
//! loaded for the principal, the gate and the seal, the compiled statement, hydration — against a
//! real database (spec `2026-10-19-asking-artifacts-by-what-they-are` §4.3, §4.4).
//!
//! The SQL's own witnesses, conjunct by conjunct, are `temper-substrate`'s
//! `find_resources_with_artifact_asks.rs`; this file holds what only the pipeline can show: that
//! the compiled statement answers with the resources owning matching data, that a shape amended
//! between `prepare` and `run_composition` changes nothing about the answer, and that the selection's
//! trace entry carries what each of its asks counted, assembled from the seal and the statement.

use std::collections::BTreeSet;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::artifact_ask::{ArtifactAsk, ArtifactPredicate};
use temper_core::types::ids::ShapeId;
use temper_core::types::query::filter::PropertyOp;
use temper_core::types::query::{
    shapes_to_load, ActInvocation, ActName, ArtifactAskPopulation, Composition, Intention,
    OutcomeDeclaration, QueryResponse, RefusalReason, ReturnSpec, ShapeInForce, StageInput,
    StageName, StageNode, StageOutput, StageRelation, StageTrace,
};
use temper_services::backend::query_read::{prepare, run_composition};
use temper_services::services::shape_loader;
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, ArtifactIntent, EnforcementMode, KindOwner};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CommitDataArtifactParams, CreateParams, DeclareShapeParams};
use uuid::Uuid;

const MEASUREMENT: &str = "measurement";

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

/// One context home owned by the system profile, its `measurement` shape declared, and three
/// resources whose bodies all match the downstream question: R1 owns an artifact in milliseconds,
/// R2 one in seconds, R3 none.
struct World {
    owner: ProfileId,
    emitter: EntityId,
    home: AnchorRef,
    shape: Uuid,
    r1: Uuid,
    r2: Uuid,
    r3: Uuid,
}

fn schema(with_p99: bool) -> serde_json::Value {
    let mut properties = json!({ "unit": { "type": "string" } });
    if with_p99 {
        properties["p99"] = json!({ "type": "number" });
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": properties
    })
}

async fn declare(
    pool: &PgPool,
    home: AnchorRef,
    owner: ProfileId,
    emitter: EntityId,
    with_p99: bool,
) -> Uuid {
    writes::declare_shape(
        pool,
        DeclareShapeParams {
            home,
            kind: MEASUREMENT,
            kind_owner: Some(KindOwner::Profile(owner.uuid())),
            schema: &schema(with_p99),
            enforcement: EnforcementMode::Advisory,
            emitter,
        },
    )
    .await
    .unwrap()
    .uuid()
}

async fn world(pool: &PgPool) -> World {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let context: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'asked', 'asked') RETURNING id",
    )
    .bind(owner.uuid())
    .fetch_one(pool)
    .await
    .unwrap();
    let home = AnchorRef::context(ContextId::from(context));
    let shape = declare(pool, home, owner, emitter, false).await;

    let mut made = Vec::new();
    for title in ["r1-milliseconds", "r2-seconds", "r3-no-artifact"] {
        let origin = format!("test://query-artifact-ask/{title}");
        let id = writes::create_resource_deferred_with(
            pool,
            CreateParams {
                idempotency_key: None,
                sources: vec![],
                title,
                origin_uri: &origin,
                body: "kestrel latency measurements",
                doc_type: "research",
                home,
                owner,
                originator: owner,
                emitter,
                properties: &[],
                chunks: None,
            },
            temper_substrate::events::EventContext::default(),
        )
        .await
        .unwrap()
        .uuid();
        made.push(id);
    }
    let w = World {
        owner,
        emitter,
        home,
        shape,
        r1: made[0],
        r2: made[1],
        r3: made[2],
    };
    for (on, unit) in [(w.r1, "ms"), (w.r2, "s")] {
        writes::commit_data_artifact(
            pool,
            CommitDataArtifactParams {
                resource: ResourceId::from(on),
                kind: MEASUREMENT,
                kind_owner: Some(KindOwner::Profile(owner.uuid())),
                intent: ArtifactIntent::Member,
                precedence: 0.0,
                content: &json!({ "unit": unit }),
                supersedes: &[],
                emitter,
            },
        )
        .await
        .unwrap();
    }
    w
}

/// An ask of `shape` that `unit` contains `value`.
fn unit_is(shape: Uuid, value: &str) -> ArtifactAsk {
    ArtifactAsk {
        shape: ShapeId::from(shape),
        predicates: vec![ArtifactPredicate {
            path: vec!["unit".to_string()],
            op: PropertyOp::Contains {
                values: vec![json!(value)],
            },
        }],
        intent: vec![],
        include_folded: false,
        shape_states: vec![],
    }
}

/// A selection asking `unit` contains `"ms"` of `shape`, piped into a `find-exact` over the
/// bodies, which is returned — a selection orders nothing and is not returnable itself.
fn asking_plan(shape: Uuid) -> Composition {
    plan_asking(vec![unit_is(shape, "ms")])
}

/// A selection asking `asks`, piped into a returned `find-exact` over the bodies.
fn plan_asking(asks: Vec<ArtifactAsk>) -> Composition {
    let sel = StageName::parse("sel").unwrap();
    let hits = StageName::parse("hits").unwrap();
    Composition {
        outcome: OutcomeDeclaration {
            returns: vec![ReturnSpec {
                stage: hits.clone(),
                with: vec![],
            }],
        },
        stages: vec![
            StageNode::Act(ActInvocation {
                name: sel.clone(),
                act: ActName::FindResourcesWith,
                intention: None,
                inputs: vec![],
                terms: Default::default(),
                resource_filter: None,
                edge_filter: None,
                properties: vec![],
                artifact_filter: asks,
            }),
            StageNode::Act(ActInvocation {
                name: hits,
                act: ActName::FindExact,
                intention: Some(Intention {
                    query: "kestrel".to_string(),
                    embedding: None,
                }),
                inputs: vec![StageInput::Upstream {
                    relation: StageRelation::Bound,
                    stage: sel,
                }],
                terms: Default::default(),
                resource_filter: None,
                edge_filter: None,
                properties: vec![],
                artifact_filter: vec![],
            }),
        ],
    }
}

/// What the handler does before `prepare`: the shapes the plan's asks name, loaded for the
/// principal.
async fn shapes_for(
    pool: &PgPool,
    principal: ProfileId,
    c: &Composition,
) -> temper_core::types::artifact_ask::ShapeContext {
    let ids = shapes_to_load(c).expect("within the request caps");
    shape_loader::load(pool, principal, &ids)
        .await
        .expect("the loader reads")
}

/// The selection stage's trace entry.
fn sel_trace(r: &QueryResponse) -> &StageTrace {
    r.trace
        .stages
        .iter()
        .find(|t| t.stage == StageName::parse("sel").unwrap())
        .expect("the selection is traced")
}

fn returned(r: &QueryResponse) -> BTreeSet<Uuid> {
    match &r.returned[&StageName::parse("hits").unwrap()].produced {
        StageOutput::Resources { hits } => hits.iter().map(|h| h.resource.id.uuid()).collect(),
        other => panic!("expected resources, got {other:?}"),
    }
}

/// **The compiled statement answers with the resources owning matching data.** All three bodies
/// match the downstream question; only R1 owns an artifact in milliseconds, so only R1 comes back.
/// Bites the asks dropped anywhere between the seal and the statement: all three come back.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_composition_asking_artifacts_answers_with_the_resources_owning_matching_data(
    pool: PgPool,
) {
    let w = world(&pool).await;
    let c = asking_plan(w.shape);
    let shapes = shapes_for(&pool, w.owner, &c).await;
    let v = prepare(c, &shapes)
        .await
        .expect("the ask lies inside its shape");
    let r = run_composition(&pool, w.owner, &v).await.expect("runs");
    assert_eq!(returned(&r), BTreeSet::from([w.r1]));
    assert!(!returned(&r).contains(&w.r2) && !returned(&r).contains(&w.r3));
}

/// **A shape amended between `prepare` and `run_composition` answers over the validated
/// population, without refusing.** The plan is prepared against the shape's first version; the
/// shape is then amended, which folds that version and mints a new id. The prepared plan still
/// runs, and still answers R1: the statement reads the home and family from the folded row by id,
/// so the population is the same lineage. Nothing refuses mid-flight.
///
/// The control shows the amendment took: preparing the same plan afresh now refuses
/// `shape_superseded`. Bites a population keyed on "resolves to this id" rather than the row's
/// home and family: the run after the amendment answers nothing.
///
/// **The trace says both versions.** The selection's one entry names the version the plan was
/// validated against (the first) and the version in force when it ran (the amendment, version 2),
/// and its population is not emptied by the amendment: both artifacts are still counted, now
/// unchecked against the new version, and R1's still matches. Bites the entry's validated version
/// read from the population row instead of the seal (in `query_read::artifact_ask_traces`,
/// `shape_version: ask.shape().shape_version` → `shape_version:
/// row.in_force_shape_version.unwrap_or_default()`): it reports 2 as validated.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_shape_amended_after_prepare_answers_over_the_validated_population(pool: PgPool) {
    let w = world(&pool).await;
    let c = asking_plan(w.shape);
    let shapes = shapes_for(&pool, w.owner, &c).await;
    let v = prepare(c.clone(), &shapes)
        .await
        .expect("the ask lies inside its shape");

    let amended = declare(&pool, w.home, w.owner, w.emitter, true).await;
    assert_ne!(amended, w.shape, "an amendment mints a new id");
    let folded: bool =
        sqlx::query_scalar("SELECT is_folded FROM kb_data_artifact_shapes WHERE id = $1")
            .bind(w.shape)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(folded, "the validated version is folded");

    let r = run_composition(&pool, w.owner, &v)
        .await
        .expect("nothing refuses mid-flight");
    assert_eq!(returned(&r), BTreeSet::from([w.r1]));
    assert!(
        r.trace.stages.iter().all(|t| t.refusal.is_none()),
        "no stage refused: {:?}",
        r.trace.stages
    );
    let entries = &sel_trace(&r).artifact_asks;
    assert_eq!(entries.len(), 1, "one entry for the one ask: {entries:?}");
    let e = &entries[0];
    assert_eq!(
        (e.position, e.shape_id, e.shape_version),
        (0, ShapeId::from(w.shape), 1),
        "the version the plan was validated against"
    );
    assert_eq!(
        e.in_force,
        Some(ShapeInForce {
            shape_id: ShapeId::from(amended),
            shape_version: 2,
        }),
        "the version in force when it ran"
    );
    assert_eq!(
        e.population,
        ArtifactAskPopulation {
            declared_satisfied: 0,
            declared_not_satisfied: 0,
            declared_not_yet_checked: 2,
            matched: 1,
        },
        "the lineage is still counted after the amendment"
    );

    let fresh = shapes_for(&pool, w.owner, &c).await;
    let refusals = prepare(c, &fresh)
        .await
        .expect_err("the version the plan names is no longer in force");
    assert!(
        refusals
            .iter()
            .any(|r| r.reason == RefusalReason::ShapeSuperseded),
        "{refusals:?}"
    );
}

/// A world whose R3 also owns an artifact in seconds: one in milliseconds (R1), two in seconds
/// (R2, R3).
async fn world_with_two_in_seconds(pool: &PgPool) -> World {
    let w = world(pool).await;
    writes::commit_data_artifact(
        pool,
        CommitDataArtifactParams {
            resource: ResourceId::from(w.r3),
            kind: MEASUREMENT,
            kind_owner: Some(KindOwner::Profile(w.owner.uuid())),
            intent: ArtifactIntent::Member,
            precedence: 0.0,
            content: &json!({ "unit": "s" }),
            supersedes: &[],
            emitter: w.emitter,
        },
    )
    .await
    .unwrap();
    w
}

/// **The trace carries one entry per sealed ask, in filter order, end to end.** The selection
/// asks `unit` contains `"ms"` and then `unit` contains `"s"`; through the real load, `prepare`
/// and `run_composition`, its trace entry carries two entries at positions 0 and 1, each naming the
/// shape and version it was validated against and in force, and each counting its own match — one
/// artifact in milliseconds, two in seconds — over the same three-artifact population.
///
/// Bites the entries read in another order (`asks.iter().rev().enumerate()` in
/// `query_read::artifact_ask_traces`): position 0 then carries the second ask's count.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn each_sealed_ask_is_traced_in_filter_order_through_the_whole_read_path(pool: PgPool) {
    let w = world_with_two_in_seconds(&pool).await;
    let c = plan_asking(vec![unit_is(w.shape, "ms"), unit_is(w.shape, "s")]);
    let shapes = shapes_for(&pool, w.owner, &c).await;
    let v = prepare(c, &shapes)
        .await
        .expect("both asks lie inside their shape");
    let r = run_composition(&pool, w.owner, &v).await.expect("runs");

    let entries = &sel_trace(&r).artifact_asks;
    let got: Vec<(usize, i64)> = entries
        .iter()
        .map(|e| (e.position, e.population.matched))
        .collect();
    assert_eq!(got, vec![(0, 1), (1, 2)], "{entries:?}");
    for e in entries {
        assert_eq!((e.shape_id, e.shape_version), (ShapeId::from(w.shape), 1));
        assert_eq!(
            e.in_force,
            Some(ShapeInForce {
                shape_id: ShapeId::from(w.shape),
                shape_version: 1,
            })
        );
        assert_eq!(
            e.population.declared_satisfied, 3,
            "each ask counts the whole governed population: {e:?}"
        );
    }
}

/// **Per-ask counts do not combine.** The same two asks AND together on one resource, and no
/// resource owns both an artifact in milliseconds and one in seconds, so the stage selects
/// nothing — while the first ask matched one artifact and the second two. Neither their sum nor
/// their minimum is what the stage produced; each ask reports its own population, and the empty
/// stage is explained by the two counts side by side rather than by either one.
///
/// Bites a population arm whose `matched` is the stage's own output (in `query_plan.rs`'s
/// `population_arms`, `c.matched FROM {call} AS c` written `(SELECT count(*) FROM "{s}")::bigint
/// FROM {call} AS c`): both entries then report 0.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_stage_with_two_asks_reports_per_ask_counts_that_do_not_combine(pool: PgPool) {
    let w = world_with_two_in_seconds(&pool).await;
    let c = plan_asking(vec![unit_is(w.shape, "ms"), unit_is(w.shape, "s")]);
    let shapes = shapes_for(&pool, w.owner, &c).await;
    let v = prepare(c, &shapes)
        .await
        .expect("both asks lie inside their shape");
    let r = run_composition(&pool, w.owner, &v).await.expect("runs");

    let sel = sel_trace(&r);
    assert_eq!(sel.produced_ids, 0, "no resource satisfies both asks");
    let matched: Vec<i64> = sel
        .artifact_asks
        .iter()
        .map(|e| e.population.matched)
        .collect();
    assert_eq!(matched, vec![1, 2], "each ask's own match");
    assert_ne!(matched.iter().sum::<i64>(), sel.produced_ids);
    assert_ne!(matched.iter().min().copied(), Some(sel.produced_ids));
    assert!(
        returned(&r).is_empty(),
        "the empty stage bounds the next to nothing"
    );
}
