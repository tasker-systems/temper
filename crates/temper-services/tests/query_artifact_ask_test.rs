#![cfg(feature = "test-db")]

//! **A composition asking data artifacts, through the whole `/api/query` read path** — the shapes
//! loaded for the principal, the gate and the seal, the compiled statement, hydration — against a
//! real database (spec `2026-10-19-asking-artifacts-by-what-they-are` §4.3, §4.4).
//!
//! The SQL's own witnesses, conjunct by conjunct, are `temper-substrate`'s
//! `find_resources_with_artifact_asks.rs`; this file holds what only the pipeline can show: that
//! the compiled statement answers with the resources owning matching data, and that a shape
//! amended between `prepare` and `run_composition` changes nothing about the answer.

use std::collections::BTreeSet;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::artifact_ask::{ArtifactAsk, ArtifactPredicate};
use temper_core::types::ids::ShapeId;
use temper_core::types::query::filter::PropertyOp;
use temper_core::types::query::{
    shapes_to_load, ActInvocation, ActName, Composition, Intention, OutcomeDeclaration,
    QueryResponse, RefusalReason, ReturnSpec, StageInput, StageName, StageNode, StageOutput,
    StageRelation,
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

/// A selection asking `unit` contains `"ms"` of `shape`, piped into a `find-exact` over the
/// bodies, which is returned — a selection orders nothing and is not returnable itself.
fn asking_plan(shape: Uuid) -> Composition {
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
                artifact_filter: vec![ArtifactAsk {
                    shape: ShapeId::from(shape),
                    predicates: vec![ArtifactPredicate {
                        path: vec!["unit".to_string()],
                        op: PropertyOp::Contains {
                            values: vec![json!("ms")],
                        },
                    }],
                    intent: vec![],
                    include_folded: false,
                    shape_states: vec![],
                }],
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
