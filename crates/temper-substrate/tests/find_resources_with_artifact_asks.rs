#![cfg(feature = "test-db")]
//! **`find-resources-with` narrows by what a resource's data artifacts hold** (spec
//! `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md` §4.1, §4.4, §6),
//! against a real database. Migration `20261104000000` adds the counted set
//! `__temper_ungated_artifact_ask_population` and the 14-argument selection core whose asks
//! conjunct reads it; this file is the witness for `matching-data-narrows-to-owning-resources`
//! and for the deny direction of §6.3.
//!
//! **The asks are bound as the composable arm binds them** — the seal's `ValidatedAsk::bind`, one
//! per ask, in a jsonb array — never hand-written JSON, so a test here exercises the shape the
//! compiler emits. The one exception says so: the bind the raw caller's ask would produce.
//!
//! **The deny direction, conjunct by conjunct.** Each §6.1 conjunct the counted set applies is
//! removed in turn from a test-only variant of the deployed body — read from `pg_proc`, the one
//! conjunct cut, created as a `pg_temp` function — and the fixture only that conjunct excludes comes
//! back through the variant and stays out through the real function. Building the variant from
//! the deployed body means the bite cannot drift from the code it bites. Visibility lives in the
//! governance relation beneath it, so its bite is a chain of such variants. The two other conjuncts
//! that live there (live artifacts only, and the family an erasure renames away) are bitten by their
//! fixture instead: the artifact counted before it was folded or renamed, and not after.
//!
//! **What the statement reads, not how it is planned.** The counted set runs its query by
//! `EXECUTE`, so an outer `EXPLAIN` sees only a function scan. The two plan tests count the rows the
//! transaction read from `kb_data_artifacts` (`pg_stat_xact_user_tables`), which sees nested
//! execution, around one run of the compiled statement — in both directions: a small home in a
//! large family, and a small family in a large home.
//!
//! **What each ask counted.** Migration `20261105000000` adds `__temper_ungated_artifact_ask_counts`,
//! one row per ask over the same counted set, and the compiled statement reads it in a `population`
//! arm per ask. The last section holds its witnesses — unverdicted content still matching and
//! counted, a narrowed ask still counting what it excluded, the deny direction for counts (the
//! owner gate, and the floor and home check the counts inherit), exactly one row, the version in
//! force after an amendment, every count in its own column — and the executor's refusal of a row
//! class it was not taught. Their bites are variants of the deployed bodies, built as above.
//!
//! Every resource is created through the real write path with its embedding deferred, so no model
//! is needed; the background rows of the plan tests are fixture inserts, as in
//! `artifact_ask_parity.rs`.

mod common;

use std::collections::BTreeSet;

use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool, Row};
use temper_core::types::artifact_ask::{
    ArtifactAsk, ArtifactPredicate, LoadedShape, ShapeContext, ShapeLoad,
};
use temper_core::types::ids::ShapeId;
use temper_core::types::query::filter::{OrdOp, PropertyOp};
use temper_core::types::query::{
    validate, ActInvocation, ActName, Composition, Intention, OutcomeDeclaration, ReturnSpec,
    StageInput, StageName, StageNode, StageRelation,
};
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, DataArtifactId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, ArtifactIntent, EnforcementMode, KindOwner};
use temper_substrate::readback::query_exec::{execute, PopulationRow, QueryRows};
use temper_substrate::readback::query_plan::{compile, CompiledQuery, QueryBind};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{
    self, CommitDataArtifactParams, CreateMode, CreateParams, DeclareShapeParams,
};
use uuid::Uuid;

// ── Harness ─────────────────────────────────────────────────────────────────────────────────────

const MEASUREMENT: &str = "measurement";

const POPULATION: &str = "__temper_ungated_artifact_ask_population";
const POPULATION_SIGNATURE: &str = "(p_visible_ids uuid[], p_reader uuid, p_ask jsonb) \
     RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int, \
     shape_state text, matched boolean)";

const GOVERNED_IN: &str = "__temper_ungated_governed_artifacts_in";
const GOVERNED_IN_SIGNATURE: &str = "(p_visible_ids uuid[], p_home_anchor_table text, \
     p_home_anchor_id uuid, p_kind_owner_table text, p_kind_owner_id uuid, p_artifact_kind text) \
     RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int, \
     content_hash text)";

const STATES_IN: &str = "__temper_ungated_governed_artifact_states_in";
const STATES_IN_SIGNATURE: &str = "(p_visible_ids uuid[], p_home_anchor_table text, \
     p_home_anchor_id uuid, p_kind_owner_table text, p_kind_owner_id uuid, p_artifact_kind text) \
     RETURNS TABLE (artifact_id uuid, resource_id uuid, shape_id uuid, shape_version int, \
     content_hash text, shape_state text)";

const COUNTS: &str = "__temper_ungated_artifact_ask_counts";
const COUNTS_SIGNATURE: &str = "(p_visible_ids uuid[], p_reader uuid, p_ask jsonb) \
     RETURNS TABLE (in_force_shape_id uuid, in_force_shape_version int, \
     declared_satisfied bigint, declared_not_satisfied bigint, \
     declared_not_yet_checked bigint, matched bigint)";

const CORE: &str = "__temper_ungated_find_resources_with";
const CORE_SIGNATURE: &str = "(p_visible_ids uuid[], p_doc_types text[], p_tags text[], \
     p_facets jsonb, p_stage text, p_status text, p_owner_profile uuid, p_owner_handle text, \
     p_title_contains text, p_artifact_asks jsonb, p_anchor_table varchar, p_anchor_id uuid, \
     p_anchor_reader uuid, p_properties jsonb) RETURNS TABLE (resource_id uuid)";

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

/// The schema the asked shape declares: an object whose `unit` is a required string and whose
/// `count` is an integer.
fn shape_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "unit": { "type": "string" },
            "count": { "type": "integer" }
        },
        "required": ["unit"]
    })
}

/// One context home, owned by the system profile P, with P's `measurement` family's shape declared
/// in it.
struct World {
    owner: ProfileId,
    emitter: EntityId,
    home: AnchorRef,
    family: (KindOwner, &'static str),
    shape: Uuid,
}

async fn world(pool: &PgPool) -> World {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = AnchorRef::context(ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), "asked", "asked")
            .await
            .unwrap(),
    ));
    let family = (KindOwner::Profile(owner.uuid()), MEASUREMENT);
    world_in(pool, owner, emitter, home, family).await
}

/// A world in `home` for `family`: the family's shape declared there.
async fn world_in(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: AnchorRef,
    family: (KindOwner, &'static str),
) -> World {
    let shape = writes::declare_shape(
        pool,
        DeclareShapeParams {
            home,
            kind: family.1,
            kind_owner: Some(family.0),
            schema: &shape_schema(),
            enforcement: EnforcementMode::Advisory,
            emitter,
        },
    )
    .await
    .unwrap()
    .uuid();
    World {
        owner,
        emitter,
        home,
        family,
        shape,
    }
}

fn params<'a>(w: &World, title: &'a str, origin: &'a str) -> CreateParams<'a> {
    CreateParams {
        idempotency_key: None,
        title,
        origin_uri: origin,
        body: "fixture body",
        doc_type: "research",
        home: w.home,
        owner: w.owner,
        originator: w.owner,
        emitter: w.emitter,
        properties: &[],
        chunks: None,
        sources: vec![],
    }
}

/// A resource homed in the world's home, its embedding deferred.
async fn resource(pool: &PgPool, w: &World, title: &str) -> Uuid {
    let origin = format!("test://artifact-asks/{title}");
    writes::create_resource_deferred_with(pool, params(w, title, &origin), EventContext::default())
        .await
        .unwrap()
        .uuid()
}

/// Commit one artifact of the world's family to `on`, superseding `supersedes`.
async fn commit_superseding(
    pool: &PgPool,
    w: &World,
    on: Uuid,
    content: Value,
    supersedes: &[DataArtifactId],
) -> DataArtifactId {
    writes::commit_data_artifact(
        pool,
        CommitDataArtifactParams {
            resource: ResourceId::from(on),
            kind: w.family.1,
            kind_owner: Some(w.family.0),
            intent: ArtifactIntent::Member,
            precedence: 0.0,
            content: &content,
            supersedes,
            emitter: w.emitter,
        },
    )
    .await
    .unwrap()
}

async fn commit(pool: &PgPool, w: &World, on: Uuid, content: Value) -> DataArtifactId {
    commit_superseding(pool, w, on, content, &[]).await
}

/// A predicate at a one-segment path.
fn at(path: &str, op: PropertyOp) -> ArtifactPredicate {
    ArtifactPredicate {
        path: vec![path.to_string()],
        op,
    }
}

fn contains(value: Value) -> PropertyOp {
    PropertyOp::Contains {
        values: vec![value],
    }
}

fn greater_than(value: i64) -> PropertyOp {
    PropertyOp::Compare {
        direction: OrdOp::Gt,
        value: json!(value),
    }
}

/// The caller's ask of `shape`: these predicates, and nothing else said.
fn ask_of(shape: Uuid, predicates: Vec<ArtifactPredicate>) -> ArtifactAsk {
    ArtifactAsk {
        shape: ShapeId::from(shape),
        predicates,
        intent: vec![],
        include_folded: false,
        shape_states: vec![],
    }
}

/// The ask as the composable arm binds it: validated against the shape's schema through a session,
/// then `ValidatedAsk::bind`. Home and family are not in the bind — the SQL reads them by id.
fn bound(ask: &ArtifactAsk) -> Value {
    let loaded = LoadedShape {
        shape_id: ask.shape,
        shape_version: 1,
        home_anchor_table: "kb_contexts".to_string(),
        home_anchor_id: Uuid::nil(),
        kind_owner_table: "kb_profiles".to_string(),
        kind_owner_id: Uuid::nil(),
        artifact_kind: MEASUREMENT.to_string(),
        schema: shape_schema(),
    };
    let shapes: ShapeContext = [(loaded.shape_id, ShapeLoad::Loaded(loaded))]
        .into_iter()
        .collect();
    let mut session = shapes.session();
    session
        .validate(ask)
        .expect("the fixture's ask lies inside its shape")
        .bind()
}

/// A stage's asks as the compiler binds them: an array of binds, in filter order.
fn asks(list: &[&ArtifactAsk]) -> Value {
    Value::Array(list.iter().map(|a| bound(a)).collect())
}

fn ids(list: &[Uuid]) -> BTreeSet<Uuid> {
    list.iter().copied().collect()
}

/// The resources a 14-argument selection core selects, called with only the visible set, the asks,
/// the reader and the open-key slot — every other narrowing NULL. `core` is the deployed function
/// or a `pg_temp` variant of it.
async fn select_through(
    conn: &mut PgConnection,
    core: &str,
    visible: Option<Vec<Uuid>>,
    artifact_asks: Option<Value>,
    reader: Uuid,
    properties: Option<Value>,
) -> BTreeSet<Uuid> {
    // `core` is one of this file's literals; every value is bound.
    let sql = format!(
        "SELECT resource_id FROM {core}($1::uuid[], NULL::text[], NULL::text[], NULL::jsonb, \
         NULL::text, NULL::text, NULL::uuid, NULL::text, NULL::text, $2::jsonb, NULL::varchar, \
         NULL::uuid, $3::uuid, $4::jsonb)"
    );
    sqlx::query_scalar::<_, Uuid>(&sql)
        .bind(visible)
        .bind(artifact_asks)
        .bind(reader)
        .bind(properties)
        .fetch_all(&mut *conn)
        .await
        .unwrap()
        .into_iter()
        .collect()
}

async fn select(
    pool: &PgPool,
    visible: &[Uuid],
    artifact_asks: Option<Value>,
    reader: ProfileId,
) -> BTreeSet<Uuid> {
    let mut conn = pool.acquire().await.unwrap();
    select_through(
        &mut conn,
        CORE,
        Some(visible.to_vec()),
        artifact_asks,
        reader.uuid(),
        None,
    )
    .await
}

/// One counted-set row: the artifact, its resource, its shape state, and whether it matched.
type Counted = (Uuid, Uuid, String, bool);

/// The counted set of one ask, through `population` — the deployed function or a variant.
async fn counted_through(
    conn: &mut PgConnection,
    population: &str,
    visible: Option<Vec<Uuid>>,
    reader: Uuid,
    ask: &Value,
) -> Vec<Counted> {
    let sql = format!(
        "SELECT artifact_id, resource_id, shape_state, matched \
           FROM {population}($1::uuid[], $2::uuid, $3::jsonb) ORDER BY artifact_id"
    );
    sqlx::query_as::<_, Counted>(&sql)
        .bind(visible)
        .bind(reader)
        .bind(ask)
        .fetch_all(&mut *conn)
        .await
        .unwrap()
}

async fn counted(
    pool: &PgPool,
    visible: Option<Vec<Uuid>>,
    reader: ProfileId,
    ask: &Value,
) -> Vec<Counted> {
    let mut conn = pool.acquire().await.unwrap();
    counted_through(&mut conn, POPULATION, visible, reader.uuid(), ask).await
}

fn artifacts_of(rows: &[Counted]) -> BTreeSet<Uuid> {
    rows.iter().map(|(artifact, ..)| *artifact).collect()
}

/// A `pg_temp` copy of `function`'s deployed body (the overload with `nargs` arguments, in its own
/// language) with the
/// one occurrence of `cut` replaced by `with`, created on `conn` as `name`. The cut must occur
/// exactly once, so a body that moved under this test fails here, by name, rather than biting
/// nothing.
async fn variant(
    conn: &mut PgConnection,
    function: &str,
    nargs: i16,
    signature: &str,
    name: &str,
    cut: &str,
    with: &str,
) {
    let (body, language): (String, String) = sqlx::query_as(
        "SELECT p.prosrc, l.lanname::text FROM pg_proc p JOIN pg_language l ON l.oid = p.prolang \
          WHERE p.proname = $1 AND p.pronargs = $2",
    )
    .bind(function)
    .bind(nargs)
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(
        body.matches(cut).count(),
        1,
        "`{cut}` must appear exactly once in {function}'s deployed body:\n{body}"
    );
    let edited = body.replacen(cut, with, 1);
    // `name` and `signature` are this file's literals; the language and the body are the deployed
    // function's, the body edited here.
    sqlx::query(&format!(
        "CREATE FUNCTION pg_temp.{name}{signature} LANGUAGE {language} STABLE \
         AS $variant${edited}$variant$"
    ))
    .execute(&mut *conn)
    .await
    .unwrap();
}

// ── H4: the asks narrow to the resources owning matching data ───────────────────────────────────

/// **A home with R1 (one matching artifact), R2 (one that does not match) and R3 (none) selects
/// exactly R1.** The ask is `unit` contains `"ms"`. Through the deployed core the answer is {R1};
/// through a variant with the asks conjunct made vacuous (`p_artifact_asks IS NULL OR` → `true OR`)
/// it is {R1, R2, R3} — so the narrowing is the conjunct's, not the fixture's. The widened gated
/// wrapper answers the same for the principal, from its own visibility verdict.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn matching_data_narrows_to_the_owning_resources(pool: PgPool) {
    let w = world(&pool).await;
    let r1 = resource(&pool, &w, "r1-matches").await;
    let r2 = resource(&pool, &w, "r2-does-not").await;
    let r3 = resource(&pool, &w, "r3-has-none").await;
    commit(&pool, &w, r1, json!({ "unit": "ms", "count": 20 })).await;
    commit(&pool, &w, r2, json!({ "unit": "s", "count": 5 })).await;
    let ask = ask_of(w.shape, vec![at("unit", contains(json!("ms")))]);
    let visible = vec![r1, r2, r3];

    assert_eq!(
        select(&pool, &visible, Some(asks(&[&ask])), w.owner).await,
        ids(&[r1])
    );

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        CORE,
        14,
        CORE_SIGNATURE,
        "core_without_asks",
        "(p_artifact_asks IS NULL OR",
        "(true OR",
    )
    .await;
    assert_eq!(
        select_through(
            &mut conn,
            "pg_temp.core_without_asks",
            Some(visible.clone()),
            Some(asks(&[&ask])),
            w.owner.uuid(),
            None,
        )
        .await,
        ids(&[r1, r2, r3]),
        "the bite: without the asks conjunct the whole visible home comes back"
    );

    // The gated wrapper, widened: its own verdict, the principal as the reader.
    let through_wrapper: BTreeSet<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "SELECT resource_id FROM query_find_resources_with($1, NULL, NULL, NULL, NULL, NULL, NULL, \
         NULL, NULL, $2::jsonb, NULL, NULL, NULL::jsonb)",
    )
    .bind(w.owner.uuid())
    .bind(asks(&[&ask]))
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .collect();
    assert_eq!(through_wrapper, ids(&[r1]));
}

/// **Predicates within one ask hold of one artifact; separate asks may be met by different
/// artifacts.** R_same owns one artifact `{unit: "ms", count: 20}`; R_split owns `{unit: "ms",
/// count: 1}` and `{unit: "s", count: 20}`. One ask with both predicates selects R_same alone; the
/// same predicates as two asks select both. Bites a matcher that tests each predicate against any
/// of the resource's artifacts: the one-ask form then selects R_split too.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn predicates_in_one_ask_hold_of_one_artifact_and_asks_of_any(pool: PgPool) {
    let w = world(&pool).await;
    let same = resource(&pool, &w, "same-artifact").await;
    let split = resource(&pool, &w, "split-artifacts").await;
    commit(&pool, &w, same, json!({ "unit": "ms", "count": 20 })).await;
    commit(&pool, &w, split, json!({ "unit": "ms", "count": 1 })).await;
    commit(&pool, &w, split, json!({ "unit": "s", "count": 20 })).await;
    let visible = vec![same, split];
    let ms = at("unit", contains(json!("ms")));
    let many = at("count", greater_than(10));

    let one_ask = ask_of(w.shape, vec![ms.clone(), many.clone()]);
    assert_eq!(
        select(&pool, &visible, Some(asks(&[&one_ask])), w.owner).await,
        ids(&[same]),
        "both predicates must hold of the same artifact"
    );

    let first = ask_of(w.shape, vec![ms]);
    let second = ask_of(w.shape, vec![many]);
    assert_eq!(
        select(&pool, &visible, Some(asks(&[&first, &second])), w.owner).await,
        ids(&[same, split]),
        "two asks may each be met by a different artifact of one resource"
    );
}

/// **An ask with no predicates matches every artifact its shape governs** — the vacuous
/// conjunction, said on purpose — so it selects every resource owning one: R1 and R2, not R3.
/// That holds because the bind writes every list out. The caller's own ask, serialized, carries no
/// `predicates`, `intent` or `shape_states` key at all, and bound that way it selects nothing: the
/// guard on binding the validated form.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_ask_with_no_predicates_matches_every_governed_artifact(pool: PgPool) {
    let w = world(&pool).await;
    let r1 = resource(&pool, &w, "r1").await;
    let r2 = resource(&pool, &w, "r2").await;
    let r3 = resource(&pool, &w, "r3-none").await;
    commit(&pool, &w, r1, json!({ "unit": "ms" })).await;
    commit(&pool, &w, r2, json!({ "unit": "s" })).await;
    let visible = vec![r1, r2, r3];
    let everything = ask_of(w.shape, vec![]);

    assert_eq!(
        select(&pool, &visible, Some(asks(&[&everything])), w.owner).await,
        ids(&[r1, r2])
    );

    let raw = Value::Array(vec![serde_json::to_value(&everything).unwrap()]);
    assert!(
        raw[0].get("predicates").is_none(),
        "the raw ask skips the empty list"
    );
    assert_eq!(
        select(&pool, &visible, Some(raw), w.owner).await,
        BTreeSet::new(),
        "bound raw, the same ask asks nothing of anything"
    );
}

/// **The two jsonb arguments, transposed, do not answer the question.** `p_artifact_asks` and
/// `p_properties` are both jsonb, so a positional call that swapped them would type-check — which is
/// why the signature keeps them apart (decision P6). Called correctly, asks `unit ~ "ms"` and the
/// open-key `has_key doc_type` select R1; swapped, each slot reads the other's list as malformed
/// and the answer is empty, never a different plausible set.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_two_jsonb_arguments_transposed_answer_differently(pool: PgPool) {
    let w = world(&pool).await;
    let r1 = resource(&pool, &w, "r1").await;
    let r2 = resource(&pool, &w, "r2").await;
    commit(&pool, &w, r1, json!({ "unit": "ms" })).await;
    commit(&pool, &w, r2, json!({ "unit": "s" })).await;
    let visible = vec![r1, r2];
    let artifact_asks = asks(&[&ask_of(w.shape, vec![at("unit", contains(json!("ms")))])]);
    let properties = json!([{ "key": "doc_type", "op": { "op": "has_key" } }]);

    let mut conn = pool.acquire().await.unwrap();
    let right = select_through(
        &mut conn,
        CORE,
        Some(visible.clone()),
        Some(artifact_asks.clone()),
        w.owner.uuid(),
        Some(properties.clone()),
    )
    .await;
    assert_eq!(right, ids(&[r1]));

    let swapped = select_through(
        &mut conn,
        CORE,
        Some(visible),
        Some(properties),
        w.owner.uuid(),
        Some(artifact_asks),
    )
    .await;
    assert_ne!(
        swapped, right,
        "a transposed call must not answer the question"
    );
    assert_eq!(swapped, BTreeSet::new(), "and fails closed");
}

// ── H5: the deny direction, one §6.1 conjunct at a time ─────────────────────────────────────────

/// **Owner visibility: only resources in the caller's verdict are counted, and a NULL verdict
/// admits nothing.** R1's matching artifact is counted when R1 is in the visible set, not when it
/// is left out, and nothing is counted for a NULL set — through the counted set and through the
/// core. The visibility join lives in the governance relation the counted set calls
/// (`__temper_ungated_governed_artifacts_in`), so the bite is a chain of variants: that relation
/// with the join cut, its states form calling the cut one, and the counted set calling that. Through
/// it, the NULL set and the set without R1 both count R1's artifact.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn owner_visibility_admits_only_the_verdict_and_null_admits_nothing(pool: PgPool) {
    let w = world(&pool).await;
    let r1 = resource(&pool, &w, "r1-visible-or-not").await;
    let r2 = resource(&pool, &w, "r2-always-visible").await;
    let x = commit(&pool, &w, r1, json!({ "unit": "ms" })).await;
    let ask = bound(&ask_of(w.shape, vec![at("unit", contains(json!("ms")))]));

    let seen = counted(&pool, Some(vec![r1, r2]), w.owner, &ask).await;
    assert_eq!(
        artifacts_of(&seen),
        BTreeSet::from([x.uuid()]),
        "the control"
    );
    assert!(counted(&pool, Some(vec![r2]), w.owner, &ask)
        .await
        .is_empty());
    assert!(counted(&pool, None, w.owner, &ask).await.is_empty());
    assert_eq!(
        select(&pool, &[r2], Some(Value::Array(vec![ask.clone()])), w.owner).await,
        BTreeSet::new()
    );

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        GOVERNED_IN,
        6,
        GOVERNED_IN_SIGNATURE,
        "governed_without_visibility",
        "JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = h.resource_id",
        "",
    )
    .await;
    variant(
        &mut conn,
        STATES_IN,
        6,
        STATES_IN_SIGNATURE,
        "states_without_visibility",
        "__temper_ungated_governed_artifacts_in(",
        "pg_temp.governed_without_visibility(",
    )
    .await;
    variant(
        &mut conn,
        POPULATION,
        3,
        POPULATION_SIGNATURE,
        "population_without_visibility",
        "__temper_ungated_governed_artifact_states_in(",
        "pg_temp.states_without_visibility(",
    )
    .await;
    for visible in [None, Some(vec![r2])] {
        let leaked = counted_through(
            &mut conn,
            "pg_temp.population_without_visibility",
            visible.clone(),
            w.owner.uuid(),
            &ask,
        )
        .await;
        assert!(
            artifacts_of(&leaked).contains(&x.uuid()),
            "the bite: without the join, R1 is counted for {visible:?}"
        );
    }
}

/// **The shape's home is re-checked readable in the statement.** A principal who is handed R1 in
/// its visible set but cannot read the shape's home counts nothing and selects nothing; the owner,
/// who can, counts R1's artifact. Through a variant whose readability check always answers yes, the
/// outsider counts it too: the bite.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_shapes_home_must_be_readable_to_the_reader(pool: PgPool) {
    let w = world(&pool).await;
    let outsider = ProfileId::from(common::insert_profile(&pool, "artifact-asks-outsider").await);
    let r1 = resource(&pool, &w, "r1").await;
    let x = commit(&pool, &w, r1, json!({ "unit": "ms" })).await;
    let ask = bound(&ask_of(w.shape, vec![at("unit", contains(json!("ms")))]));

    assert_eq!(
        artifacts_of(&counted(&pool, Some(vec![r1]), w.owner, &ask).await),
        BTreeSet::from([x.uuid()]),
        "the control: the home's owner reads it"
    );
    assert!(counted(&pool, Some(vec![r1]), outsider, &ask)
        .await
        .is_empty());
    assert_eq!(
        select(
            &pool,
            &[r1],
            Some(Value::Array(vec![ask.clone()])),
            outsider
        )
        .await,
        BTreeSet::new(),
        "R1 is in the outsider's visible set and still not selected"
    );

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        POPULATION,
        3,
        POPULATION_SIGNATURE,
        "population_without_home_check",
        "anchor_readable_by_profile(p_reader,",
        "true OR anchor_readable_by_profile(p_reader,",
    )
    .await;
    let leaked = counted_through(
        &mut conn,
        "pg_temp.population_without_home_check",
        Some(vec![r1]),
        outsider.uuid(),
        &ask,
    )
    .await;
    assert!(
        artifacts_of(&leaked).contains(&x.uuid()),
        "the bite: without the re-check, the outsider counts under a shape it cannot read"
    );
}

/// **Live artifacts only.** R4's matching artifact is counted, and R4 selected, until a commit
/// supersedes it with one that does not match; then the folded artifact is not counted and R4 is
/// not selected. The fold is the only thing that changed, so the governance relation's live-only
/// conjunct is what excludes it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_folded_artifact_is_not_counted(pool: PgPool) {
    let w = world(&pool).await;
    let r4 = resource(&pool, &w, "r4-folds").await;
    let first = commit(&pool, &w, r4, json!({ "unit": "ms" })).await;
    let ask = bound(&ask_of(w.shape, vec![at("unit", contains(json!("ms")))]));
    let stage = Value::Array(vec![ask.clone()]);

    assert_eq!(
        artifacts_of(&counted(&pool, Some(vec![r4]), w.owner, &ask).await),
        BTreeSet::from([first.uuid()]),
        "the control: counted while live"
    );
    assert_eq!(
        select(&pool, &[r4], Some(stage.clone()), w.owner).await,
        ids(&[r4])
    );

    let successor = commit_superseding(&pool, &w, r4, json!({ "unit": "s" }), &[first]).await;
    let after = counted(&pool, Some(vec![r4]), w.owner, &ask).await;
    assert_eq!(
        artifacts_of(&after),
        BTreeSet::from([successor.uuid()]),
        "the folded artifact left the count; its successor is counted"
    );
    assert!(after.iter().all(|(.., matched)| !matched));
    assert_eq!(
        select(&pool, &[r4], Some(stage), w.owner).await,
        BTreeSet::new()
    );
}

/// **An erased resource is excluded twice over, and each half bites on its own.** Erasure sets
/// `is_active = false` and renames every artifact's family to `erased:<asserted_by_event_id>`
/// (`20261008100000`, step 9f); each fixture here applies one of the two.
///
/// - R5 is deactivated and keeps its family: not counted, and through a variant with the
///   `kb_resources_live` floor cut, counted — the bite.
/// - R6 keeps `is_active` and has its artifact renamed as erasure renames it: counted before, not
///   after — the governance relation's family match is what excludes it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_erased_resource_is_excluded_by_is_active_and_by_the_family_rename(pool: PgPool) {
    let w = world(&pool).await;
    let r5 = resource(&pool, &w, "r5-deactivated").await;
    let r6 = resource(&pool, &w, "r6-renamed").await;
    let x5 = commit(&pool, &w, r5, json!({ "unit": "ms" })).await;
    let x6 = commit(&pool, &w, r6, json!({ "unit": "ms" })).await;
    let ask = bound(&ask_of(w.shape, vec![at("unit", contains(json!("ms")))]));
    let both = Some(vec![r5, r6]);

    assert_eq!(
        artifacts_of(&counted(&pool, both.clone(), w.owner, &ask).await),
        BTreeSet::from([x5.uuid(), x6.uuid()]),
        "the control: both counted before either half of erasure"
    );

    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(r5)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE kb_data_artifacts SET artifact_kind = 'erased:' || asserted_by_event_id::text \
          WHERE id = $1",
    )
    .bind(x6.uuid())
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        counted(&pool, both.clone(), w.owner, &ask).await.is_empty(),
        "neither is counted"
    );

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        POPULATION,
        3,
        POPULATION_SIGNATURE,
        "population_without_floor",
        "JOIN kb_resources_live lv ON lv.id = g.resource_id",
        "",
    )
    .await;
    let leaked = counted_through(
        &mut conn,
        "pg_temp.population_without_floor",
        both,
        w.owner.uuid(),
        &ask,
    )
    .await;
    assert_eq!(
        artifacts_of(&leaked),
        BTreeSet::from([x5.uuid()]),
        "the bite: without the floor the deactivated resource is counted; the renamed family \
         stays out on its own"
    );
}

/// **A resource still ingesting is outside the count.** A segmented begin births R7
/// `in_progress`; its matching artifact is not counted and R7 is not selected, though R7 is in the
/// visible set. Through the variant with the `kb_resources_live` floor cut, it is counted: the bite.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_resource_still_ingesting_is_not_counted(pool: PgPool) {
    let w = world(&pool).await;
    let origin = "test://artifact-asks/r7-ingesting";
    let r7 = writes::create_resource_with_mode(
        &pool,
        params(&w, "r7-ingesting", origin),
        EventContext::default(),
        CreateMode {
            defer: true,
            segmented: true,
        },
    )
    .await
    .unwrap()
    .uuid();
    let state: String = sqlx::query_scalar("SELECT ingest_state FROM kb_resources WHERE id = $1")
        .bind(r7)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "in_progress", "the fixture is an interrupted ingest");
    let x = commit(&pool, &w, r7, json!({ "unit": "ms" })).await;
    let ask = bound(&ask_of(w.shape, vec![at("unit", contains(json!("ms")))]));

    assert!(counted(&pool, Some(vec![r7]), w.owner, &ask)
        .await
        .is_empty());
    assert_eq!(
        select(&pool, &[r7], Some(Value::Array(vec![ask.clone()])), w.owner).await,
        BTreeSet::new()
    );

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        POPULATION,
        3,
        POPULATION_SIGNATURE,
        "population_without_floor",
        "JOIN kb_resources_live lv ON lv.id = g.resource_id",
        "",
    )
    .await;
    let leaked = counted_through(
        &mut conn,
        "pg_temp.population_without_floor",
        Some(vec![r7]),
        w.owner.uuid(),
        &ask,
    )
    .await;
    assert_eq!(
        artifacts_of(&leaked),
        BTreeSet::from([x.uuid()]),
        "the bite: without the floor the ingesting resource is counted"
    );
}

// ── What the compiled statement reads ───────────────────────────────────────────────────────────

/// The most `kb_data_artifacts` rows one call of the counted set may read over these fixtures,
/// measured on its own through `__temper_ungated_artifact_ask_counts` ([`counts_rows_read`]): the
/// home's few artifacts, each read once through governance and once by id for its intent, and
/// nothing of the family or the home beyond them. One call reads 14 rows over the small home and
/// 16 over the small family; the bound is twice the larger, so a call that more than doubles the
/// larger fixture's reads fails, while a walk of either fixture reads thousands.
const ONE_CALL_BOUND: i64 = 32;

/// The most `kb_data_artifacts` rows one run of the compiled statement may read for one ask.
///
/// **The arithmetic: 2 calls × [`ONE_CALL_BOUND`].** The statement calls the counted set twice for
/// the ask — in the selection's asks conjunct and in the ask's population arm — with the same three
/// arguments (the hoisted visible set, the principal, the ask as element 0 of one bind). One call
/// over these fixtures reads at most [`ONE_CALL_BOUND`], which the counts-alone measurement holds,
/// so two read at most twice that.
/// Against a family of 9,000 outside the home, or a home of 18,000, a walk of either reads
/// thousands, so the bound separates the two plans by more than an order of magnitude.
const TWO_CALL_BOUND: i64 = 2 * ONE_CALL_BOUND;

const BACKGROUND_HOMES: i32 = 150;
const BACKGROUND_RESOURCES: i32 = 3_000;
const ARTIFACTS_PER_BACKGROUND_RESOURCE: i32 = 6;

/// Rows this transaction has read from `kb_data_artifacts` so far: tuples returned by sequential
/// scans plus heap tuples fetched by index and bitmap scans. A fetch is counted before any filter
/// runs, so an index walk that returns few rows and discards the rest counts every row it touched.
async fn artifact_rows_read(conn: &mut PgConnection) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(seq_tup_read, 0) + COALESCE(idx_tup_fetch, 0) \
           FROM pg_stat_xact_user_tables WHERE relname = 'kb_data_artifacts'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap()
}

/// The compiled statement for one composition: a selection stage asking `asks` of the world's
/// shape, piped into a returned `find-exact` over the bodies — validated against the shape as the
/// handler loads it (its first version), and compiled by `compile` as `run_composition` compiles
/// it.
fn compiled_plan(w: &World, asks: Vec<ArtifactAsk>) -> CompiledQuery {
    let loaded = LoadedShape {
        shape_id: ShapeId::from(w.shape),
        shape_version: 1,
        home_anchor_table: "kb_contexts".to_string(),
        home_anchor_id: w.home.id,
        kind_owner_table: "kb_profiles".to_string(),
        kind_owner_id: w.owner.uuid(),
        artifact_kind: w.family.1.to_string(),
        schema: shape_schema(),
    };
    let shapes: ShapeContext = [(loaded.shape_id, ShapeLoad::Loaded(loaded))]
        .into_iter()
        .collect();
    let sel = StageName::parse("sel").unwrap();
    let hits = StageName::parse("hits").unwrap();
    let c = Composition {
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
                    query: "fixture".to_string(),
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
    };
    let v = validate(&c, &mut shapes.session()).expect("the ask lies inside its shape");
    compile(&v, w.owner).expect("the plan compiles")
}

/// One run of a compiled statement on `conn`, its binds bound as `query_exec::execute` binds them,
/// answering the ids of the returned `hits` stage.
async fn run_compiled(conn: &mut PgConnection, q: &CompiledQuery) -> BTreeSet<Uuid> {
    let mut query = sqlx::query(&q.sql);
    for bind in &q.binds {
        query = match bind {
            QueryBind::Profile(p) => query.bind(p.uuid()),
            QueryBind::Uuids(ids) => query.bind(ids.clone()),
            QueryBind::Text(t) => query.bind(t.clone()),
            QueryBind::Int(i) => query.bind(*i),
            QueryBind::Texts(t) => query.bind(t.clone()),
            QueryBind::Id(u) => query.bind(*u),
            QueryBind::Json(j) => query.bind(j.clone()),
            QueryBind::Embedding(_) => panic!("the fixture plan binds no vector"),
        };
    }
    query
        .fetch_all(&mut *conn)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| {
            row.get::<String, _>("row_class") == "hit" && row.get::<String, _>("stage") == "hits"
        })
        .map(|row| row.get::<Uuid, _>("id"))
        .collect()
}

/// One run of the compiled statement for `ask`, and how many `kb_data_artifacts` rows it read,
/// measured inside one transaction around that run alone. The count sees the counted set's
/// `EXECUTE`, which an outer `EXPLAIN` cannot.
///
/// **The whole statement, population arm included.** The statement calls the counted set twice
/// for the one ask: once in the selection's asks conjunct, once in the ask's population arm
/// (`__temper_ungated_artifact_ask_counts`). The arm is asserted present here, so a bound measured
/// over the run is a bound on both calls.
async fn compiled_rows_read(pool: &PgPool, w: &World, ask: ArtifactAsk) -> (BTreeSet<Uuid>, i64) {
    let q = compiled_plan(w, vec![ask]);
    assert_eq!(
        q.sql
            .matches(&format!("{COUNTS}((SELECT ids FROM __temper_vis),"))
            .count(),
        1,
        "one population arm, for the one ask, in:\n{}",
        q.sql
    );
    let mut tx = pool.begin().await.unwrap();
    let before = artifact_rows_read(&mut tx).await;
    let answered = run_compiled(&mut tx, &q).await;
    let read = artifact_rows_read(&mut tx).await - before;
    tx.rollback().await.unwrap();
    (answered, read)
}

/// The counts function alone for `ask`, called as the population arm calls it — the principal's
/// own visible set, the principal as reader, the ask's bind — and how many `kb_data_artifacts` rows
/// that one call read, measured as [`compiled_rows_read`] measures the statement.
async fn counts_rows_read(pool: &PgPool, w: &World, ask: &ArtifactAsk) -> (CountsRow, i64) {
    let mut tx = pool.begin().await.unwrap();
    let before = artifact_rows_read(&mut tx).await;
    let row: CountsRow = sqlx::query_as(
        "SELECT in_force_shape_id, in_force_shape_version, declared_satisfied, \
                declared_not_satisfied, declared_not_yet_checked, matched \
           FROM __temper_ungated_artifact_ask_counts(\
                (SELECT array_agg(resource_id) FROM resources_visible_to($1)), $1, $2::jsonb)",
    )
    .bind(w.owner.uuid())
    .bind(bound(ask))
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let read = artifact_rows_read(&mut tx).await - before;
    tx.rollback().await.unwrap();
    (row, read)
}

async fn analyze(pool: &PgPool) {
    sqlx::query(
        "ANALYZE kb_resources, kb_resource_homes, kb_data_artifacts, kb_data_artifact_shapes, \
                 kb_data_artifact_verdicts, kb_data_artifact_content",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn any_event(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// **A small home in a large family is answered from the home, never by walking the family.**
///
/// One small home holds R1 (three matching artifacts) and R2 (one that does not match); 150 other
/// homes hold 3,000 resources with six artifacts each, half of them in the asked family — 9,000
/// artifacts of the family outside the home (asserted). As in `artifact_ask_parity.rs`, the other
/// half has a fresh owner per row, so `n_distinct(kind_owner_id)` is large: read from the shape row
/// inside a statement, the family is estimated at about one row, and a plan starting from it walks
/// all of it once per home resource.
///
/// The statement is the one production runs: a composition whose selection stage asks the fixture
/// shape, piped into a returned `find-exact`, compiled by `compile` and run with its binds — the
/// hoisted visible set (every resource is the owner's) and the sealed asks included. Asserted on
/// the rows the transaction read from `kb_data_artifacts` around that one run
/// (`pg_stat_xact_user_tables`, so the counted set's `EXECUTE` is seen), never timings:
///
/// 1. the run answers {R1};
/// 2. it read at least one row — the count sees the read at all, so the bound cannot pass by
///    measuring nothing;
/// 3. it read at most [`TWO_CALL_BOUND`] — the counted set called twice (the asks conjunct and the
///    population arm) over the home's four artifacts and the lookups by id beside them, where a
///    walk of the family reads thousands;
/// 4. the counts function alone, called as the population arm calls it, counts the home's four
///    artifacts with three matching and reads at least one row and at most [`ONE_CALL_BOUND`];
/// 5. the statement's reads beyond that call — the selection's asks conjunct, the only other reader
///    of `kb_data_artifacts` in it — are at most [`ONE_CALL_BOUND`] too. Each half of the statement
///    is within its own bound, so one half regressing to twice its reads is not hidden by the other
///    staying cheap.
///
/// Bites the counted set reverted to `LANGUAGE sql` reading the shape row in-statement (its home
/// and family no longer values when the artifact query is planned): the family walk reads about
/// 18,000 rows and assertions 3 and 4 go red. And bites a population arm correlated to every
/// visible resource (in `query_plan.rs`'s `population_arms`, `FROM {call} AS c` written
/// `FROM unnest({VISIBLE_IDS}) AS each_visible CROSS JOIN LATERAL {call} AS c`, with the ask
/// written `((${n}::jsonb -> {k}) || jsonb_build_object('v', each_visible))` so the call depends on
/// the row and is not computed once): the home is re-read 3,002 times and assertion 3 goes red.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_small_home_in_a_large_family_is_not_answered_by_walking_the_family(pool: PgPool) {
    let w = world(&pool).await;
    let r1 = resource(&pool, &w, "plan-r1").await;
    let r2 = resource(&pool, &w, "plan-r2").await;
    for count in [1, 2, 3] {
        commit(&pool, &w, r1, json!({ "unit": "ms", "count": count })).await;
    }
    commit(&pool, &w, r2, json!({ "unit": "s" })).await;

    sqlx::query(
        "WITH homes AS (
             SELECT n, gen_random_uuid() AS anchor_id FROM generate_series(0, $2 - 1) AS n
         ), made AS (
             INSERT INTO kb_resources (title, origin_uri)
             SELECT 'background ' || g, 'test://artifact-asks/background/' || g
               FROM generate_series(1, $3) AS g
             RETURNING id
         )
         INSERT INTO kb_resource_homes
                (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         SELECT m.id, 'kb_contexts', h.anchor_id, $1, $1
           FROM (SELECT id, row_number() OVER (ORDER BY id) AS rn FROM made) AS m
           JOIN homes AS h ON h.n = m.rn % $2",
    )
    .bind(w.owner.uuid())
    .bind(BACKGROUND_HOMES)
    .bind(BACKGROUND_RESOURCES)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_data_artifacts
                (id, resource_id, kind_owner_table, kind_owner_id, artifact_kind, intent,
                 content_hash, content_bytes, asserted_by_event_id, last_event_id)
         SELECT gen_random_uuid(), r.id,
                CASE WHEN k % 2 = 0 THEN 'kb_profiles' ELSE 'kb_teams' END,
                CASE WHEN k % 2 = 0 THEN $1 ELSE gen_random_uuid() END,
                'measurement', 'member', 'background', 0, $2, $2
           FROM kb_resources r
          CROSS JOIN generate_series(1, $3) AS k
          WHERE r.origin_uri LIKE 'test://artifact-asks/background/%'",
    )
    .bind(w.owner.uuid())
    .bind(any_event(&pool).await)
    .bind(ARTIFACTS_PER_BACKGROUND_RESOURCE)
    .execute(&pool)
    .await
    .unwrap();
    analyze(&pool).await;

    let family_outside: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_data_artifacts
          WHERE kind_owner_table = 'kb_profiles' AND kind_owner_id = $1
            AND artifact_kind = 'measurement' AND resource_id NOT IN ($2, $3)",
    )
    .bind(w.owner.uuid())
    .bind(r1)
    .bind(r2)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        family_outside, 9_000,
        "the family is large outside the home"
    );

    let ask = ask_of(w.shape, vec![at("unit", contains(json!("ms")))]);
    let (answered, read) = compiled_rows_read(&pool, &w, ask.clone()).await;
    assert_eq!(answered, ids(&[r1]));
    assert!(read > 0, "the count sees the call's reads");
    assert!(
        read <= TWO_CALL_BOUND,
        "the compiled statement read {read} rows of kb_data_artifacts for a home holding four, \
         beside a family of {family_outside}"
    );

    let (counts, one_call) = counts_rows_read(&pool, &w, &ask).await;
    assert_eq!(
        by_state(counts),
        (4, 0, 0, 3),
        "the home's four, three matching"
    );
    assert!(one_call > 0, "the count sees the counts call's reads");
    assert!(
        one_call <= ONE_CALL_BOUND,
        "the counts call alone read {one_call} rows of kb_data_artifacts for a home holding four, \
         beside a family of {family_outside}"
    );
    let conjunct = read - one_call;
    assert!(
        conjunct <= ONE_CALL_BOUND,
        "the selection's asks conjunct read {conjunct} rows of kb_data_artifacts for a home \
         holding four, beside a family of {family_outside}"
    );
}

/// **A small family in a large home is answered from the family, never by walking the home.** The
/// other direction of the same claim: the planner starts from whichever is really smaller.
///
/// One large context home holds 3,000 resources with six artifacts each, every one of another
/// family (a fresh team owner per row) — 18,000 artifacts in the home (asserted) — and three
/// resources each owning one artifact of the asked family, `tiny`, whose shape is declared there.
/// Run as above, through the compiled statement, and asserted on the rows the transaction read
/// from `kb_data_artifacts` around that run, population arm included: it answers the three, read at
/// least one row, and read at most [`TWO_CALL_BOUND`] — two calls about the family, where reaching
/// the artifacts through the home's resources reads thousands. The counts function alone, called
/// as the population arm calls it, counts the three and reads at least one row and at most
/// [`ONE_CALL_BOUND`], and the statement's reads beyond it (the selection's asks conjunct) are at
/// most [`ONE_CALL_BOUND`] too.
///
/// Bites the family made unindexable in `__temper_ungated_governed_artifacts_in` — e.g. its
/// `a.artifact_kind = p_artifact_kind` join condition written `a.artifact_kind || '' =
/// p_artifact_kind` — so the plan can only reach artifacts through the home's resources: the read
/// count climbs toward the home's 18,000 and both bounds go red. The correlated population arm
/// described on the test above bites the whole-statement bound here too.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_small_family_in_a_large_home_is_not_answered_by_walking_the_home(pool: PgPool) {
    let first = world(&pool).await;
    let big = AnchorRef::context(ContextId::from(
        common::insert_context(&pool, "kb_profiles", first.owner.uuid(), "large", "large")
            .await
            .unwrap(),
    ));
    let w = world_in(
        &pool,
        first.owner,
        first.emitter,
        big,
        (KindOwner::Profile(first.owner.uuid()), "tiny"),
    )
    .await;
    let mut tiny = Vec::new();
    for title in ["tiny-1", "tiny-2", "tiny-3"] {
        let on = resource(&pool, &w, title).await;
        commit(&pool, &w, on, json!({ "unit": "ms" })).await;
        tiny.push(on);
    }

    sqlx::query(
        "WITH made AS (
             INSERT INTO kb_resources (title, origin_uri)
             SELECT 'large home ' || g, 'test://artifact-asks/large-home/' || g
               FROM generate_series(1, $3) AS g
             RETURNING id
         )
         INSERT INTO kb_resource_homes
                (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         SELECT m.id, 'kb_contexts', $2, $1, $1 FROM made AS m",
    )
    .bind(w.owner.uuid())
    .bind(w.home.id)
    .bind(BACKGROUND_RESOURCES)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_data_artifacts
                (id, resource_id, kind_owner_table, kind_owner_id, artifact_kind, intent,
                 content_hash, content_bytes, asserted_by_event_id, last_event_id)
         SELECT gen_random_uuid(), r.id, 'kb_teams', gen_random_uuid(), 'measurement', 'member',
                'background', 0, $1, $1
           FROM kb_resources r
          CROSS JOIN generate_series(1, $2) AS k
          WHERE r.origin_uri LIKE 'test://artifact-asks/large-home/%'",
    )
    .bind(any_event(&pool).await)
    .bind(ARTIFACTS_PER_BACKGROUND_RESOURCE)
    .execute(&pool)
    .await
    .unwrap();
    analyze(&pool).await;

    let in_home: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_data_artifacts a
           JOIN kb_resource_homes h ON h.resource_id = a.resource_id
          WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = $1",
    )
    .bind(w.home.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        in_home, 18_003,
        "the home is large and the family is three of it"
    );

    let ask = ask_of(w.shape, vec![at("unit", contains(json!("ms")))]);
    let (answered, read) = compiled_rows_read(&pool, &w, ask.clone()).await;
    assert_eq!(answered, ids(&tiny));
    assert!(read > 0, "the count sees the call's reads");
    assert!(
        read <= TWO_CALL_BOUND,
        "the compiled statement read {read} rows of kb_data_artifacts for a family of three, \
         in a home holding {in_home}"
    );

    let (counts, one_call) = counts_rows_read(&pool, &w, &ask).await;
    assert_eq!(
        by_state(counts),
        (3, 0, 0, 3),
        "the family's three, all matching"
    );
    assert!(one_call > 0, "the count sees the counts call's reads");
    assert!(
        one_call <= ONE_CALL_BOUND,
        "the counts call alone read {one_call} rows of kb_data_artifacts for a family of three, \
         in a home holding {in_home}"
    );
    let conjunct = read - one_call;
    assert!(
        conjunct <= ONE_CALL_BOUND,
        "the selection's asks conjunct read {conjunct} rows of kb_data_artifacts for a family of \
         three, in a home holding {in_home}"
    );
}

// ── What each ask counted: the population beside the stage ──────────────────────────────────────

/// A home and family with no shape declared yet: `shape` is nil until [`declare`] runs, so
/// artifacts committed first carry no verdict when the shape arrives.
async fn undeclared_world(pool: &PgPool) -> World {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = AnchorRef::context(ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), "asked", "asked")
            .await
            .unwrap(),
    ));
    World {
        owner,
        emitter,
        home,
        family: (KindOwner::Profile(owner.uuid()), MEASUREMENT),
        shape: Uuid::nil(),
    }
}

/// Declare the world's family's shape in its home with `schema` — its first declaration, or an
/// amendment of the shape in force, which folds that row and mints a new id.
async fn declare(pool: &PgPool, w: &World, schema: &Value) -> Uuid {
    writes::declare_shape(
        pool,
        DeclareShapeParams {
            home: w.home,
            kind: w.family.1,
            kind_owner: Some(w.family.0),
            schema,
            enforcement: EnforcementMode::Advisory,
            emitter: w.emitter,
        },
    )
    .await
    .unwrap()
    .uuid()
}

/// [`shape_schema`] with one more declared path: a different schema, so declaring it amends.
fn amended_schema() -> Value {
    let mut schema = shape_schema();
    schema["properties"]["p99"] = json!({ "type": "number" });
    schema
}

/// One row of the counts function: the in-force id and version, then the three by-state counts
/// and `matched`.
type CountsRow = (Option<Uuid>, Option<i32>, i64, i64, i64, i64);

/// Every row the counts `function` — the deployed one or a variant — returns for one ask.
async fn counts_through(
    conn: &mut PgConnection,
    function: &str,
    visible: Option<Vec<Uuid>>,
    reader: Uuid,
    ask: &Value,
) -> Vec<CountsRow> {
    // `function` is one of this file's literals; every value is bound.
    let sql = format!(
        "SELECT in_force_shape_id, in_force_shape_version, declared_satisfied, \
                declared_not_satisfied, declared_not_yet_checked, matched \
           FROM {function}($1::uuid[], $2::uuid, $3::jsonb)"
    );
    sqlx::query_as::<_, CountsRow>(&sql)
        .bind(visible)
        .bind(reader)
        .bind(ask)
        .fetch_all(&mut *conn)
        .await
        .unwrap()
}

async fn counts(
    pool: &PgPool,
    visible: Option<Vec<Uuid>>,
    reader: ProfileId,
    ask: &Value,
) -> Vec<CountsRow> {
    let mut conn = pool.acquire().await.unwrap();
    counts_through(&mut conn, COUNTS, visible, reader.uuid(), ask).await
}

/// The one row an ask's counts are, asserted to be exactly one.
fn one(rows: Vec<CountsRow>) -> CountsRow {
    assert_eq!(rows.len(), 1, "exactly one row per ask: {rows:?}");
    rows[0]
}

/// `(declared_satisfied, declared_not_satisfied, declared_not_yet_checked, matched)`.
fn by_state(row: CountsRow) -> (i64, i64, i64, i64) {
    (row.2, row.3, row.4, row.5)
}

fn population_by_state(p: &PopulationRow) -> (i64, i64, i64, i64) {
    (
        p.declared_satisfied,
        p.declared_not_satisfied,
        p.declared_not_yet_checked,
        p.matched,
    )
}

/// The compiled statement for a selection asking `asks`, run through `query_exec::execute` as
/// `run_composition` runs it: every row class, typed.
async fn executed(pool: &PgPool, w: &World, asks: Vec<ArtifactAsk>) -> QueryRows {
    execute(pool, &compiled_plan(w, asks))
        .await
        .expect("the statement runs")
}

fn produced(rows: &QueryRows, stage: &str) -> i64 {
    rows.tally(stage)
        .map(|t| t.produced)
        .unwrap_or_else(|| panic!("`{stage}` is tallied"))
}

/// **Content committed before its shape was declared still matches, and is counted as not yet
/// checked** (spec §5: the population is governance, never verdicts). R1 (`unit: "ms"`) and R2
/// (`unit: "s"`) commit their artifacts; then the shape is declared, and nothing reconciles them
/// (no verdict row exists, asserted). Through the compiled statement the selection still selects
/// R1 on content, and the ask's population row counts both artifacts under
/// `declared_not_yet_checked`, one matched, with the shape itself in force.
///
/// The bite is the population defined by verdict-row presence: a chain of variants whose
/// governance states relation INNER-joins the verdicts (`LEFT JOIN kb_data_artifact_verdicts v` →
/// `JOIN kb_data_artifact_verdicts v`), read by the counted set, read by the selection core and by
/// the counts. Through it the stage selects nothing and the counts are zeros — the test's two
/// assertions above, red.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn unverdicted_content_still_matches_and_is_counted_as_not_yet_checked(pool: PgPool) {
    let mut w = undeclared_world(&pool).await;
    let r1 = resource(&pool, &w, "r1-ms").await;
    let r2 = resource(&pool, &w, "r2-s").await;
    commit(&pool, &w, r1, json!({ "unit": "ms" })).await;
    commit(&pool, &w, r2, json!({ "unit": "s" })).await;
    w.shape = declare(&pool, &w, &shape_schema()).await;
    let verdicts: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_verdicts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        verdicts, 0,
        "the fixture is reconciliation that has not run"
    );
    let ask = ask_of(w.shape, vec![at("unit", contains(json!("ms")))]);

    let rows = executed(&pool, &w, vec![ask.clone()]).await;
    assert_eq!(produced(&rows, "sel"), 1, "the stage matches on content");
    assert_eq!(rows.populations.len(), 1, "one row for the one ask");
    let p = &rows.populations[0];
    assert_eq!((p.stage.as_str(), p.position), ("sel", 0));
    assert_eq!(
        population_by_state(p),
        (0, 0, 2, 1),
        "both artifacts are counted unchecked, and the content match is counted"
    );
    assert_eq!(
        (p.in_force_shape_id, p.in_force_shape_version),
        (Some(w.shape), Some(1))
    );

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        STATES_IN,
        6,
        STATES_IN_SIGNATURE,
        "states_by_verdict",
        "LEFT JOIN kb_data_artifact_verdicts v",
        "JOIN kb_data_artifact_verdicts v",
    )
    .await;
    variant(
        &mut conn,
        POPULATION,
        3,
        POPULATION_SIGNATURE,
        "population_by_verdict",
        "__temper_ungated_governed_artifact_states_in(",
        "pg_temp.states_by_verdict(",
    )
    .await;
    variant(
        &mut conn,
        CORE,
        14,
        CORE_SIGNATURE,
        "core_by_verdict",
        "__temper_ungated_artifact_ask_population(",
        "pg_temp.population_by_verdict(",
    )
    .await;
    variant(
        &mut conn,
        COUNTS,
        3,
        COUNTS_SIGNATURE,
        "counts_by_verdict",
        "__temper_ungated_artifact_ask_population(",
        "pg_temp.population_by_verdict(",
    )
    .await;
    assert_eq!(
        select_through(
            &mut conn,
            "pg_temp.core_by_verdict",
            Some(vec![r1, r2]),
            Some(asks(&[&ask])),
            w.owner.uuid(),
            None,
        )
        .await,
        BTreeSet::new(),
        "the bite: a population of verdicts selects nothing while reconciliation lags"
    );
    assert_eq!(
        by_state(one(counts_through(
            &mut conn,
            "pg_temp.counts_by_verdict",
            Some(vec![r1, r2]),
            w.owner.uuid(),
            &bound(&ask),
        )
        .await)),
        (0, 0, 0, 0),
        "and counts nothing, so the lag would read as absence"
    );
}

/// **An ask narrowed by verdict state returns less and still names what it excluded.** The early
/// artifact was committed before the shape (unchecked); the late one after it, conforming, so the
/// commit verdicts it `declared_satisfied`. Asked with every state, the stage selects both; asked
/// for `declared_satisfied` alone, it selects one — and its counts still show the unchecked row,
/// with `matched` the narrowed figure.
///
/// Bites the counted set applying `shape_states` as a row filter rather than inside `matched` (a
/// variant whose `WHERE` gains `$7->'shape_states' @> jsonb_build_array(g.shape_state)`, read by
/// the counts): the narrowed counts lose the unchecked row, and the narrowness reads as absence.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_narrowed_ask_leaves_the_rows_it_excluded_counted(pool: PgPool) {
    let mut w = undeclared_world(&pool).await;
    let early = resource(&pool, &w, "early-unchecked").await;
    commit(&pool, &w, early, json!({ "unit": "ms" })).await;
    w.shape = declare(&pool, &w, &shape_schema()).await;
    let late = resource(&pool, &w, "late-satisfied").await;
    commit(&pool, &w, late, json!({ "unit": "ms" })).await;
    let all = ask_of(w.shape, vec![at("unit", contains(json!("ms")))]);
    let narrowed = ArtifactAsk {
        shape_states: vec!["declared_satisfied".to_string()],
        ..all.clone()
    };

    let wide = executed(&pool, &w, vec![all]).await;
    let narrow = executed(&pool, &w, vec![narrowed.clone()]).await;
    assert_eq!(produced(&wide, "sel"), 2);
    assert_eq!(
        produced(&narrow, "sel"),
        1,
        "the narrowed ask returns fewer"
    );
    assert_eq!(
        population_by_state(&wide.populations[0]),
        (1, 0, 1, 2),
        "the fixture: one satisfied, one unchecked, both matching"
    );
    assert_eq!(
        population_by_state(&narrow.populations[0]),
        (1, 0, 1, 1),
        "the excluded unchecked row is still counted; matched is the narrowed figure"
    );

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        POPULATION,
        3,
        POPULATION_SIGNATURE,
        "population_state_filtered",
        "WHERE jsonb_typeof($7->'intent') = 'array'",
        "WHERE $7->'shape_states' @> jsonb_build_array(g.shape_state) \
         AND jsonb_typeof($7->'intent') = 'array'",
    )
    .await;
    variant(
        &mut conn,
        COUNTS,
        3,
        COUNTS_SIGNATURE,
        "counts_state_filtered",
        "__temper_ungated_artifact_ask_population(",
        "pg_temp.population_state_filtered(",
    )
    .await;
    assert_eq!(
        by_state(one(counts_through(
            &mut conn,
            "pg_temp.counts_state_filtered",
            Some(vec![early, late]),
            w.owner.uuid(),
            &bound(&narrowed),
        )
        .await)),
        (1, 0, 0, 1),
        "the bite: with the state narrowing as a row filter the unchecked row leaves the counts"
    );
}

/// **The counts range over the caller's verdict, and a NULL verdict counts nothing** (spec §6.3:
/// a count computed with the owner gate removed must differ). R1 is in the visible set and R2 is
/// not; each owns a matching artifact. The gated counts see R1's alone; a NULL set counts zeros.
/// The version in force still names the shape — it is about the shape's home, which the owner
/// reads, not about the population.
///
/// The bite is the owner gate removed: the governance relation's visibility join cut, read through
/// its states form, the counted set and the counts. Through that chain R2's artifact is counted,
/// for the visible set and for NULL alike, so the two answers differ.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_counts_range_over_the_callers_verdict_and_a_null_verdict_counts_nothing(pool: PgPool) {
    let w = world(&pool).await;
    let r1 = resource(&pool, &w, "r1-visible").await;
    let r2 = resource(&pool, &w, "r2-invisible").await;
    commit(&pool, &w, r1, json!({ "unit": "ms" })).await;
    commit(&pool, &w, r2, json!({ "unit": "ms" })).await;
    let ask = bound(&ask_of(w.shape, vec![at("unit", contains(json!("ms")))]));

    let gated = one(counts(&pool, Some(vec![r1]), w.owner, &ask).await);
    assert_eq!(by_state(gated), (1, 0, 0, 1), "R1's artifact alone");
    assert_eq!((gated.0, gated.1), (Some(w.shape), Some(1)));
    let nothing = one(counts(&pool, None, w.owner, &ask).await);
    assert_eq!(by_state(nothing), (0, 0, 0, 0), "NULL admits nothing");
    assert_eq!(
        (nothing.0, nothing.1),
        (Some(w.shape), Some(1)),
        "the version in force is the home's, readable to the owner"
    );

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        GOVERNED_IN,
        6,
        GOVERNED_IN_SIGNATURE,
        "governed_ungated",
        "JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = h.resource_id",
        "",
    )
    .await;
    variant(
        &mut conn,
        STATES_IN,
        6,
        STATES_IN_SIGNATURE,
        "states_ungated",
        "__temper_ungated_governed_artifacts_in(",
        "pg_temp.governed_ungated(",
    )
    .await;
    variant(
        &mut conn,
        POPULATION,
        3,
        POPULATION_SIGNATURE,
        "population_ungated",
        "__temper_ungated_governed_artifact_states_in(",
        "pg_temp.states_ungated(",
    )
    .await;
    variant(
        &mut conn,
        COUNTS,
        3,
        COUNTS_SIGNATURE,
        "counts_ungated",
        "__temper_ungated_artifact_ask_population(",
        "pg_temp.population_ungated(",
    )
    .await;
    for visible in [Some(vec![r1]), None] {
        let leaked = one(counts_through(
            &mut conn,
            "pg_temp.counts_ungated",
            visible.clone(),
            w.owner.uuid(),
            &ask,
        )
        .await);
        assert_eq!(
            by_state(leaked),
            (2, 0, 0, 2),
            "the bite: without the owner gate R2 is counted for {visible:?}"
        );
        assert_ne!(by_state(leaked), by_state(gated));
    }
}

/// **A home unreadable at execution counts nothing and names no version in force.** The outsider
/// holds R1 in its visible set but cannot read the shape's home: one row, all zeros, and a NULL
/// in-force pair. The owner, the control, reads both.
///
/// The bite is the in-force lookup's home re-check removed (`anchor_readable_by_profile(p_reader,
/// asked.home_anchor_table,` → `true OR ...`): the outsider is then told the live shape's id and
/// version under a home it cannot read. The counts stay zero through it, because the counted set
/// re-checks the home on its own.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unreadable_home_counts_nothing_and_names_no_version_in_force(pool: PgPool) {
    let w = world(&pool).await;
    let outsider = ProfileId::from(common::insert_profile(&pool, "artifact-counts-outsider").await);
    let r1 = resource(&pool, &w, "r1").await;
    commit(&pool, &w, r1, json!({ "unit": "ms" })).await;
    let ask = bound(&ask_of(w.shape, vec![at("unit", contains(json!("ms")))]));

    assert_eq!(
        one(counts(&pool, Some(vec![r1]), w.owner, &ask).await),
        (Some(w.shape), Some(1), 1, 0, 0, 1),
        "the control: the home's owner"
    );
    assert_eq!(
        one(counts(&pool, Some(vec![r1]), outsider, &ask).await),
        (None, None, 0, 0, 0, 0)
    );

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        COUNTS,
        3,
        COUNTS_SIGNATURE,
        "counts_without_home_check",
        "anchor_readable_by_profile(p_reader, asked.home_anchor_table,",
        "true OR anchor_readable_by_profile(p_reader, asked.home_anchor_table,",
    )
    .await;
    assert_eq!(
        one(counts_through(
            &mut conn,
            "pg_temp.counts_without_home_check",
            Some(vec![r1]),
            outsider.uuid(),
            &ask,
        )
        .await),
        (Some(w.shape), Some(1), 0, 0, 0, 0),
        "the bite: without the re-check the outsider is told the version in force"
    );
}

/// **An ask is always one row.** A governed shape with nothing in view counts one row of zeros,
/// with the shape in force — not a missing row — through the counts function and through the
/// compiled statement, where the empty stage still carries its population row. An ask naming no
/// shape at all counts one row of zeros and a NULL in-force pair.
///
/// Bites the counts grouped (`AS p) AS n` → `AS p GROUP BY p.shape_state) AS n`): an empty
/// population is then no row at all, and the absence would have to be read as a number.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_ask_counts_exactly_one_row_even_over_nothing(pool: PgPool) {
    let w = world(&pool).await;
    let r1 = resource(&pool, &w, "r1-owns-nothing").await;
    let everything = ask_of(w.shape, vec![]);

    assert_eq!(
        one(counts(&pool, Some(vec![r1]), w.owner, &bound(&everything)).await),
        (Some(w.shape), Some(1), 0, 0, 0, 0)
    );
    let absent = bound(&ask_of(Uuid::now_v7(), vec![]));
    assert_eq!(
        one(counts(&pool, Some(vec![r1]), w.owner, &absent).await),
        (None, None, 0, 0, 0, 0),
        "an ask naming no row"
    );

    let rows = executed(&pool, &w, vec![everything.clone()]).await;
    assert_eq!(produced(&rows, "sel"), 0, "the stage is empty");
    assert_eq!(rows.populations.len(), 1, "and its ask still has its row");
    assert_eq!(population_by_state(&rows.populations[0]), (0, 0, 0, 0));

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        COUNTS,
        3,
        COUNTS_SIGNATURE,
        "counts_grouped",
        "AS p) AS n",
        "AS p GROUP BY p.shape_state) AS n",
    )
    .await;
    assert!(
        counts_through(
            &mut conn,
            "pg_temp.counts_grouped",
            Some(vec![r1]),
            w.owner.uuid(),
            &bound(&everything),
        )
        .await
        .is_empty(),
        "the bite: grouped, an empty population is no row"
    );
}

/// **After an amendment the counts name the live version and keep the lineage.** The statement is
/// compiled against the shape's first version; the shape is then amended, which folds that row
/// and mints a new id at version 2. Run afterwards, the ask's population row names the new id and
/// version 2 in force, and still counts R1's artifact — unchecked now, since its verdict was
/// against version 1 — so the population does not empty when its shape is amended.
///
/// Bites the in-force lookup answering with the asked row rather than the live one
/// (`SELECT live.id, live.shape_version` → `SELECT asked.id, asked.shape_version`): it reports the
/// folded first version as in force.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn after_an_amendment_the_counts_name_the_live_version_and_keep_the_lineage(pool: PgPool) {
    let w = world(&pool).await;
    let r1 = resource(&pool, &w, "r1").await;
    commit(&pool, &w, r1, json!({ "unit": "ms" })).await;
    let ask = ask_of(w.shape, vec![at("unit", contains(json!("ms")))]);
    let q = compiled_plan(&w, vec![ask.clone()]);

    let amended = declare(&pool, &w, &amended_schema()).await;
    assert_ne!(amended, w.shape, "an amendment mints a new id");
    let rows = execute(&pool, &q).await.expect("the statement runs");
    let p = &rows.populations[0];
    assert_eq!(
        (p.in_force_shape_id, p.in_force_shape_version),
        (Some(amended), Some(2))
    );
    assert_eq!(
        population_by_state(p),
        (0, 0, 1, 1),
        "the lineage's artifact is still counted, unchecked against the new version"
    );
    assert_eq!(produced(&rows, "sel"), 1);

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        COUNTS,
        3,
        COUNTS_SIGNATURE,
        "counts_of_the_asked_row",
        "SELECT live.id, live.shape_version",
        "SELECT asked.id, asked.shape_version",
    )
    .await;
    let asked_row = one(counts_through(
        &mut conn,
        "pg_temp.counts_of_the_asked_row",
        Some(vec![r1]),
        w.owner.uuid(),
        &bound(&ask),
    )
    .await);
    assert_eq!(
        (asked_row.0, asked_row.1),
        (Some(w.shape), Some(1)),
        "the bite: the asked row reports the folded version as in force"
    );
}

/// **A row class the executor was not taught fails the read; it is never read as a tally.** A
/// hand-built statement in the compiled column list labels its one row `mystery`: `execute` errors
/// naming the class. The control labels the same row `tally` and reads as one.
///
/// Bites `RowClass::parse` answering an unknown value with `Tally`: the mystery row then reads as
/// a stage's disclosure numbers.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unknown_row_class_fails_the_read_rather_than_reading_as_a_tally(pool: PgPool) {
    let labelled = |class: &str| CompiledQuery {
        sql: format!(
            "SELECT '{class}'::text AS row_class, 's'::text AS stage, NULL::uuid AS id, \
             NULL::text AS kind, NULL::double precision AS quantity, NULL::jsonb AS via, \
             NULL::uuid AS region, 0::bigint AS produced, 0::bigint AS unusable, \
             NULL::bigint AS ask_position, NULL::uuid AS in_force_shape_id, \
             NULL::int AS in_force_shape_version, NULL::bigint AS declared_satisfied, \
             NULL::bigint AS declared_not_satisfied, NULL::bigint AS declared_not_yet_checked, \
             NULL::bigint AS matched"
        ),
        binds: vec![],
        cte_names: vec![],
        refusals: vec![],
    };

    let err = execute(&pool, &labelled("mystery"))
        .await
        .expect_err("an unknown class is not read");
    assert!(
        err.to_string().contains("unknown row class `mystery`"),
        "got: {err}"
    );
    let control = execute(&pool, &labelled("tally"))
        .await
        .expect("the same row, labelled a tally");
    assert_eq!(control.tallies.len(), 1);
}

/// **Every count is its own column.** One home whose artifacts are in all three states, in distinct
/// numbers: three committed before the shape was declared (`declared_not_yet_checked`), one
/// conforming committed after it (`declared_satisfied`), and two non-conforming committed after it
/// under the advisory shape (`count` a string, so `declared_not_satisfied`). The ask is `unit`
/// contains `"ms"`, which the three unchecked artifacts and the satisfied one hold. Through the
/// compiled statement and `execute`, and through the counts function directly, the population is
/// `(1, 2, 3, 4)`: each figure distinct, so a column read for another shows.
///
/// Bites the `declared_not_satisfied` FILTER literal misspelled in `20261105000000` (e.g.
/// `'declared_unsatisfied'`): that figure reads 0. And bites `execute` reading
/// `try_get("declared_satisfied")` into `declared_not_satisfied`: it reads 1.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn every_count_is_its_own_column(pool: PgPool) {
    let mut w = undeclared_world(&pool).await;
    let early = resource(&pool, &w, "early-unchecked").await;
    for count in [1, 2, 3] {
        commit(&pool, &w, early, json!({ "unit": "ms", "count": count })).await;
    }
    w.shape = declare(&pool, &w, &shape_schema()).await;
    let late = resource(&pool, &w, "late-verdicted").await;
    commit(&pool, &w, late, json!({ "unit": "ms", "count": 4 })).await;
    for count in ["five", "six"] {
        commit(&pool, &w, late, json!({ "unit": "s", "count": count })).await;
    }
    let ask = ask_of(w.shape, vec![at("unit", contains(json!("ms")))]);

    let rows = executed(&pool, &w, vec![ask.clone()]).await;
    assert_eq!(population_by_state(&rows.populations[0]), (1, 2, 3, 4));
    assert_eq!(produced(&rows, "sel"), 2);
    assert_eq!(
        by_state(one(counts(
            &pool,
            Some(vec![early, late]),
            w.owner,
            &bound(&ask)
        )
        .await)),
        (1, 2, 3, 4)
    );
}

/// **The counts inherit the ingest floor and the home check from the counted set, and each bites
/// through the counts.** R_live and R_off each own a matching artifact; R_off is then deactivated.
/// The gated counts see R_live's alone, and an outsider handed R_live counts nothing.
///
/// Both conjuncts live in `__temper_ungated_artifact_ask_population` and the counts call it, never
/// restate it, so each bite is a chain read through `counts_through`:
/// - the floor cut (`JOIN kb_resources_live lv ON lv.id = g.resource_id` removed): the counts then
///   see R_off's artifact too;
/// - the counted set's home check answering yes (`anchor_readable_by_profile(p_reader,` → `true OR
///   anchor_readable_by_profile(p_reader,`): the outsider then counts R_live's artifact, though the
///   counts' own lookup still names no version in force for it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_counts_inherit_the_floor_and_the_home_check_from_the_counted_set(pool: PgPool) {
    let w = world(&pool).await;
    let outsider = ProfileId::from(common::insert_profile(&pool, "artifact-floor-outsider").await);
    let live = resource(&pool, &w, "r-live").await;
    let off = resource(&pool, &w, "r-off").await;
    commit(&pool, &w, live, json!({ "unit": "ms" })).await;
    commit(&pool, &w, off, json!({ "unit": "ms" })).await;
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(off)
        .execute(&pool)
        .await
        .unwrap();
    let ask = bound(&ask_of(w.shape, vec![at("unit", contains(json!("ms")))]));
    let both = Some(vec![live, off]);

    assert_eq!(
        by_state(one(counts(&pool, both.clone(), w.owner, &ask).await)),
        (1, 0, 0, 1),
        "the control: the deactivated resource is outside the floor"
    );
    assert_eq!(
        one(counts(&pool, Some(vec![live]), outsider, &ask).await),
        (None, None, 0, 0, 0, 0),
        "the control: the outsider cannot read the home"
    );

    let mut conn = pool.acquire().await.unwrap();
    variant(
        &mut conn,
        POPULATION,
        3,
        POPULATION_SIGNATURE,
        "population_without_floor",
        "JOIN kb_resources_live lv ON lv.id = g.resource_id",
        "",
    )
    .await;
    variant(
        &mut conn,
        COUNTS,
        3,
        COUNTS_SIGNATURE,
        "counts_without_floor",
        "__temper_ungated_artifact_ask_population(",
        "pg_temp.population_without_floor(",
    )
    .await;
    assert_eq!(
        by_state(one(counts_through(
            &mut conn,
            "pg_temp.counts_without_floor",
            both,
            w.owner.uuid(),
            &ask
        )
        .await)),
        (2, 0, 0, 2),
        "the bite: without the floor the deactivated resource is counted"
    );

    variant(
        &mut conn,
        POPULATION,
        3,
        POPULATION_SIGNATURE,
        "population_without_home_check",
        "anchor_readable_by_profile(p_reader,",
        "true OR anchor_readable_by_profile(p_reader,",
    )
    .await;
    variant(
        &mut conn,
        COUNTS,
        3,
        COUNTS_SIGNATURE,
        "counts_without_home_check",
        "__temper_ungated_artifact_ask_population(",
        "pg_temp.population_without_home_check(",
    )
    .await;
    assert_eq!(
        one(counts_through(
            &mut conn,
            "pg_temp.counts_without_home_check",
            Some(vec![live]),
            outsider.uuid(),
            &ask,
        )
        .await),
        (None, None, 1, 0, 0, 1),
        "the bite: without the counted set's home check the outsider counts under the shape"
    );
}
