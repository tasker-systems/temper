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
use temper_core::types::artifact_ask::{ArtifactAsk, ArtifactPredicate, ShapeInForce};
use temper_core::types::ids::ShapeId;
use temper_core::types::query::filter::PropertyOp;
use temper_core::types::query::{
    shapes_to_load, ActInvocation, ActName, ArtifactAskPopulation, CombineNode, CombineOp,
    Composition, Intention, OutcomeDeclaration, QueryResponse, RefusalReason, ReturnSpec,
    StageInput, StageName, StageNode, StageOutput, StageRelation, StageTrace,
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

/// The second family the multi-ask fixtures ask, beside [`MEASUREMENT`]: one home, two families,
/// so two asks on one stage name two shapes.
const LATENCY: &str = "latency";

/// Declare `kind`'s shape (the owner's family of that name) in `home`.
async fn declare_kind(
    pool: &PgPool,
    home: AnchorRef,
    owner: ProfileId,
    emitter: EntityId,
    kind: &str,
) -> Uuid {
    writes::declare_shape(
        pool,
        DeclareShapeParams {
            home,
            kind,
            kind_owner: Some(KindOwner::Profile(owner.uuid())),
            schema: &schema(false),
            enforcement: EnforcementMode::Advisory,
            emitter,
        },
    )
    .await
    .unwrap()
    .uuid()
}

/// A context owned by `owner`, as a home.
async fn context_home(pool: &PgPool, owner: ProfileId, slug: &str) -> AnchorRef {
    let context: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner.uuid())
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap();
    AnchorRef::context(ContextId::from(context))
}

/// A resource in `home` whose body matches the downstream question.
async fn resource_in(
    pool: &PgPool,
    home: AnchorRef,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
) -> Uuid {
    let origin = format!("test://query-artifact-ask/{title}");
    writes::create_resource_deferred_with(
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
    .uuid()
}

/// Commit one artifact of the owner's `kind` family to `on`.
async fn commit_kind(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    on: Uuid,
    kind: &str,
    content: serde_json::Value,
) {
    writes::commit_data_artifact(
        pool,
        CommitDataArtifactParams {
            resource: ResourceId::from(on),
            kind,
            kind_owner: Some(KindOwner::Profile(owner.uuid())),
            intent: ArtifactIntent::Member,
            precedence: 0.0,
            content: &content,
            supersedes: &[],
            emitter,
        },
    )
    .await
    .unwrap();
}

fn in_force(shape: Uuid, version: i32) -> Option<ShapeInForce> {
    Some(ShapeInForce {
        shape_id: ShapeId::from(shape),
        shape_version: version,
    })
}

/// [`world`] plus a second shape in the same home, for the owner's `latency` family, and two
/// `latency` artifacts in seconds (on R2 and R3). The `measurement` family holds R1's artifact in
/// milliseconds and R2's in seconds.
async fn world_with_a_second_family(pool: &PgPool) -> (World, Uuid) {
    let w = world(pool).await;
    let latency = declare_kind(pool, w.home, w.owner, w.emitter, LATENCY).await;
    for on in [w.r2, w.r3] {
        commit_kind(
            pool,
            w.owner,
            w.emitter,
            on,
            LATENCY,
            json!({ "unit": "s" }),
        )
        .await;
    }
    (w, latency)
}

async fn run_as(pool: &PgPool, principal: ProfileId, c: Composition) -> QueryResponse {
    let shapes = shapes_for(pool, principal, &c).await;
    let v = prepare(c, &shapes)
        .await
        .expect("every ask lies inside its shape");
    run_composition(pool, principal, &v).await.expect("runs")
}

/// **The trace carries one entry per sealed ask, in filter order, end to end — each naming its own
/// shape.** The selection asks the `measurement` shape for `unit` contains `"ms"`, then the
/// `latency` shape (one home, another family) for `unit` contains `"s"`. Through the real load,
/// `prepare` and `run_composition`, its trace entry carries two entries: position 0 names the
/// `measurement` shape, validated and in force, and counts its one match among two artifacts;
/// position 1 names the `latency` shape and counts its two matches among two.
///
/// Bites the entries read in another order (`asks.iter().rev().enumerate()` in
/// `query_read::artifact_ask_traces`): position 0 then names the `latency` shape.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn each_sealed_ask_is_traced_in_filter_order_through_the_whole_read_path(pool: PgPool) {
    let (w, latency) = world_with_a_second_family(&pool).await;
    let r = run_as(
        &pool,
        w.owner,
        plan_asking(vec![unit_is(w.shape, "ms"), unit_is(latency, "s")]),
    )
    .await;

    let entries = &sel_trace(&r).artifact_asks;
    let got: Vec<_> = entries
        .iter()
        .map(|e| {
            (
                e.position,
                e.shape_id,
                e.shape_version,
                e.in_force,
                e.population.declared_satisfied,
                e.population.matched,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (0, ShapeId::from(w.shape), 1, in_force(w.shape, 1), 2, 1),
            (1, ShapeId::from(latency), 1, in_force(latency, 1), 2, 2),
        ],
        "{entries:?}"
    );
}

/// **Per-ask counts do not combine.** The same two asks AND together on one resource, and no
/// resource owns both a `measurement` in milliseconds and a `latency` in seconds, so the stage
/// selects nothing — while the first ask matched one artifact and the second two. Neither their
/// sum nor their minimum is what the stage produced; each ask reports its own population, and the
/// empty stage is explained by the two counts side by side rather than by either one.
///
/// Bites a population arm whose `matched` is the stage's own output (in `query_plan.rs`'s
/// `population_arms`, `c.matched FROM {call} AS c` written `(SELECT count(*) FROM "{s}")::bigint
/// FROM {call} AS c`): both entries then report 0.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_stage_with_two_asks_reports_per_ask_counts_that_do_not_combine(pool: PgPool) {
    let (w, latency) = world_with_a_second_family(&pool).await;
    let r = run_as(
        &pool,
        w.owner,
        plan_asking(vec![unit_is(w.shape, "ms"), unit_is(latency, "s")]),
    )
    .await;

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

/// **Every count is its own column, end to end.** One home whose `measurement` artifacts are in
/// all three states, in distinct numbers: three committed before the shape was declared (and never
/// reconciled), so `declared_not_yet_checked`; one conforming committed after it,
/// `declared_satisfied`; two non-conforming (`unit` a number, under an advisory shape) committed
/// after it, `declared_not_satisfied`. The ask is `unit` contains `"ms"`, which the three unchecked
/// and the one satisfied artifact hold: the entry's population is `(1, 2, 3, 4)`, each figure
/// distinct, so a column read or copied for another at any hop shows.
///
/// Bites a column swapped at each hop: the FILTER literal for `declared_not_satisfied` misspelled
/// in `20261105000000` (it reads 0); `execute` reading `try_get("declared_satisfied")` into
/// `declared_not_satisfied` (it reads 1); `artifact_ask_traces` copying `row.declared_satisfied`
/// into `declared_not_satisfied` (it reads 1).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_count_reaches_the_trace_in_its_own_column(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = context_home(&pool, owner, "four-states").await;
    let early = resource_in(&pool, home, owner, emitter, "early-unchecked").await;
    for n in 0..3 {
        commit_kind(
            &pool,
            owner,
            emitter,
            early,
            MEASUREMENT,
            json!({ "unit": "ms", "n": n }),
        )
        .await;
    }
    let shape = declare(&pool, home, owner, emitter, false).await;
    let late = resource_in(&pool, home, owner, emitter, "late-verdicted").await;
    commit_kind(
        &pool,
        owner,
        emitter,
        late,
        MEASUREMENT,
        json!({ "unit": "ms" }),
    )
    .await;
    for n in 0..2 {
        commit_kind(
            &pool,
            owner,
            emitter,
            late,
            MEASUREMENT,
            json!({ "unit": 7, "n": n }),
        )
        .await;
    }

    let r = run_as(&pool, owner, plan_asking(vec![unit_is(shape, "ms")])).await;
    let entries = &sel_trace(&r).artifact_asks;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(
        entries[0].population,
        ArtifactAskPopulation {
            declared_satisfied: 1,
            declared_not_satisfied: 2,
            declared_not_yet_checked: 3,
            matched: 4,
        }
    );
    assert_eq!(sel_trace(&r).produced_ids, 2, "both resources own a match");
}

/// **Two asking stages, two homes, joined by `union` — the plan the querying skill teaches for
/// shapes in two homes.** `sel` asks home A's `measurement` shape and `sel_b` asks home B's; each
/// stage's entry names its own shape and counts its own home: one match in A (R1), two in B. The
/// union returns all three resources.
///
/// Bites every population arm labelled with the first asking stage (in `query_plan.rs`'s
/// `population_arms`, `'{s}'::text AS stage` written `'sel'::text AS stage`): `sel_b`'s row is
/// then a second row for `sel`'s ask 0, and the response fails.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn two_asking_stages_over_two_homes_each_carry_their_own_counts(pool: PgPool) {
    let w = world(&pool).await;
    let home_b = context_home(&pool, w.owner, "asked-b").await;
    let shape_b = declare(&pool, home_b, w.owner, w.emitter, false).await;
    let mut in_b = Vec::new();
    for title in ["b1-milliseconds", "b2-milliseconds"] {
        let on = resource_in(&pool, home_b, w.owner, w.emitter, title).await;
        commit_kind(
            &pool,
            w.owner,
            w.emitter,
            on,
            MEASUREMENT,
            json!({ "unit": "ms" }),
        )
        .await;
        in_b.push(on);
    }

    let mut c = plan_asking(vec![unit_is(w.shape, "ms")]);
    let sel_b = StageName::parse("sel_b").unwrap();
    let both = StageName::parse("both").unwrap();
    c.stages.insert(
        1,
        StageNode::Act(ActInvocation {
            name: sel_b.clone(),
            act: ActName::FindResourcesWith,
            intention: None,
            inputs: vec![],
            terms: Default::default(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
            artifact_filter: vec![unit_is(shape_b, "ms")],
        }),
    );
    c.stages.insert(
        2,
        StageNode::Combine(CombineNode {
            name: both.clone(),
            op: CombineOp::Union,
            inputs: vec![StageName::parse("sel").unwrap(), sel_b],
        }),
    );
    if let StageNode::Act(hits) = &mut c.stages[3] {
        hits.inputs = vec![StageInput::Upstream {
            relation: StageRelation::Bound,
            stage: both,
        }];
    }

    let r = run_as(&pool, w.owner, c).await;
    let of = |stage: &str| -> Vec<(ShapeId, i64)> {
        r.trace
            .stages
            .iter()
            .find(|t| t.stage == StageName::parse(stage).unwrap())
            .expect("traced")
            .artifact_asks
            .iter()
            .map(|e| (e.shape_id, e.population.matched))
            .collect()
    };
    assert_eq!(of("sel"), vec![(ShapeId::from(w.shape), 1)]);
    assert_eq!(of("sel_b"), vec![(ShapeId::from(shape_b), 2)]);
    assert_eq!(
        returned(&r),
        BTreeSet::from([w.r1, in_b[0], in_b[1]]),
        "the union of the two homes' matches"
    );
}

/// **The deny direction, through the compiled statement, as a principal other than the owner.**
/// A reader holds a read grant on the shape's home, so it can ask the shape and sees every
/// resource homed there; its own run counts the home's two artifacts, one matching, with the shape
/// in force. The grant is then revoked between `prepare` and `run_composition`: the same prepared
/// plan, run as the reader, has an empty selection, a population of zeros and `in_force: null` —
/// nothing counted under a home it can no longer read — while the owner's run of the same plan
/// still counts both.
///
/// A reader who sees only some of a home's resources is not constructible: `resources_visible_to`
/// admits every resource homed in a context the principal can read (its context arm), and a
/// principal who cannot read the home is refused the shape at `prepare` (`shape_not_askable`), so
/// within one home the visible set is all or nothing. This test is the deny direction that
/// remains: the gate moving under a prepared plan.
///
/// Bites the in-force lookup's home check removed (in `20261105000000`,
/// `anchor_readable_by_profile(p_reader, asked.home_anchor_table,` → `true OR …`): the revoked
/// reader is told the version in force. And bites the arm handed a verdict and reader that are
/// not the caller's (in `query_plan.rs`'s `CoreCall::Population` rendering, `{VISIBLE_IDS},
/// {PRINCIPAL_BIND}` → `(SELECT array_agg(h.resource_id) FROM kb_resource_homes h), (SELECT
/// h.owner_profile_id FROM kb_resource_homes h LIMIT 1)`): the revoked reader then counts the
/// owner's population.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_reader_whose_home_access_is_revoked_after_prepare_counts_nothing(pool: PgPool) {
    let w = world(&pool).await;
    let reader: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind("artifact-ask-reader")
    .fetch_one(&pool)
    .await
    .unwrap();
    let reader = ProfileId::from(reader);
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, \
              granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(w.home.id)
    .bind(reader.uuid())
    .bind(w.owner.uuid())
    .execute(&pool)
    .await
    .expect("grant read on the home");

    let c = asking_plan(w.shape);
    let shapes = shapes_for(&pool, reader, &c).await;
    let v = prepare(c, &shapes)
        .await
        .expect("the reader can ask a shape in a home it reads");
    let granted = run_composition(&pool, reader, &v).await.expect("runs");
    let full = ArtifactAskPopulation {
        declared_satisfied: 2,
        declared_not_satisfied: 0,
        declared_not_yet_checked: 0,
        matched: 1,
    };
    let entry = &sel_trace(&granted).artifact_asks[0];
    assert_eq!(
        (entry.in_force, entry.population),
        (in_force(w.shape, 1), full),
        "the control: the reader's own run counts the home it reads"
    );

    sqlx::query(
        "DELETE FROM kb_access_grants \
          WHERE subject_table = 'kb_contexts' AND subject_id = $1 AND principal_id = $2",
    )
    .bind(w.home.id)
    .bind(reader.uuid())
    .execute(&pool)
    .await
    .unwrap();
    let revoked = run_composition(&pool, reader, &v)
        .await
        .expect("nothing refuses mid-flight");
    let sel = sel_trace(&revoked);
    assert_eq!(sel.produced_ids, 0, "the selection is empty");
    assert_eq!(
        (
            sel.artifact_asks[0].in_force,
            sel.artifact_asks[0].population
        ),
        (
            None,
            ArtifactAskPopulation {
                declared_satisfied: 0,
                declared_not_satisfied: 0,
                declared_not_yet_checked: 0,
                matched: 0,
            }
        ),
        "nothing counted, and no version named, under a home the reader no longer reads"
    );

    let owners = run_composition(&pool, w.owner, &v).await.expect("runs");
    assert_eq!(
        sel_trace(&owners).artifact_asks[0].population,
        full,
        "the owner's run of the same plan still counts the home"
    );
}
