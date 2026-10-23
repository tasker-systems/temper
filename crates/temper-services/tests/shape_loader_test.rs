//! Integration test — the shape loader decides, in a fixed order and through the gated read,
//! whether a shape an ask names can be asked (spec
//! `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md` §2.4, §3.5 steps
//! 1–3), and the by-id shape read carries the vocabulary an ask may use (§2.5).
//!
//! Each `ShapeLoad` arm is reached from a real fixture: a readable live shape, an unreadable one,
//! an absent one, a folded one with and without a live successor, and a stored shape whose schema
//! does not compile. A shape that does not compile is stored through the substrate's
//! `writes::declare_shape`, which does not compile — the service's declare refuses it.
//!
//! The connection count of the loopback listener a stored schema's `$ref` names guards against a
//! regression; it is not the fetch witness, because a fetching compiler's blocking fetch panics
//! inside this async runtime before it connects. The witnesses are the synchronous
//! `shape_schema_no_fetch` tests in temper-substrate, whose constructor the loader compiles
//! through.
#![cfg(feature = "test-db")]

use std::collections::BTreeSet;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::artifact_ask::walk::MAX_SCHEMA_BYTES;
use temper_core::types::artifact_ask::{
    within_schema_bound, ArtifactAsk, ArtifactPredicate, BoundKind, JsonType, PathFacts,
    ShapeContext, ShapeLoad, ShapeSuccessor, Vocabulary,
};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EntityId, ProfileId, ShapeId};
use temper_core::types::query::{PlanRefusal, PropertyOp, RefusalReason};
use temper_services::backend::substrate_read;
use temper_services::error::ApiError;
use temper_services::services::shape_loader::test_support::enumerations_tagged;
use temper_services::services::shape_loader::{load, load_one};
use temper_services::services::shape_service::{self, DeclareShapeServiceParams};
use temper_substrate::events::EventContext;
use temper_substrate::payloads::{AnchorRef, EnforcementMode, KindOwner};
use temper_substrate::readback;
use temper_substrate::scenario::bootseed;
use temper_substrate::shape_schema::test_support::compiles_tagged;
use temper_substrate::writes::{self, CreateParams, DeclareShapeParams};
use uuid::Uuid;

// The counting listener is shared with temper-substrate's tests, by path, so the suites agree on
// what a connection is.
#[path = "../../temper-substrate/tests/common/fetch_probe.rs"]
mod fetch_probe;

use fetch_probe::FetchProbe;

// ── seeding ──────────────────────────────────────────────────────────────────────────────────

struct World {
    owner: ProfileId,
    stranger: ProfileId,
    emitter: EntityId,
    context: Uuid,
}

/// One context owned by `system` holding one resource (so a declare's family owner defaults), an
/// emitter for the writes, and a stranger profile who cannot read the context.
async fn seed_world(pool: &PgPool, slug: &str) -> World {
    bootseed::seed_system(pool).await.unwrap();
    let owner: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let emitter: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'test-emitter') RETURNING id",
    )
    .bind(owner)
    .fetch_one(pool)
    .await
    .unwrap();
    let context: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap();
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: "subject",
            origin_uri: "subject",
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(context)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap();
    let stranger: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) \
         VALUES ('shape-loader-stranger', 'shape-loader-stranger') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    World {
        owner: ProfileId::from(owner),
        stranger: ProfileId::from(stranger),
        emitter: EntityId::from(emitter),
        context,
    }
}

fn value_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    })
}

/// A schema that cannot compile without a fetch: its `$ref` names the probe's listener.
fn remote_ref_schema(probe: &FetchProbe) -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": probe.url("shape.json"),
    })
}

/// Declare through the service, which compiles: a shape that can be asked.
async fn declare(pool: &PgPool, w: &World, kind: &str, schema: &serde_json::Value) -> ShapeId {
    shape_service::declare_shape(
        pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind,
            kind_owner: None,
            schema,
            enforcement: EnforcementMode::Advisory,
            principal: w.owner,
            emitter: w.emitter,
        },
    )
    .await
    .unwrap()
}

/// Store through the substrate write, which does not compile: how a shape whose schema the no-fetch
/// compiler refuses comes to be stored.
async fn store(pool: &PgPool, w: &World, kind: &str, schema: &serde_json::Value) -> ShapeId {
    writes::declare_shape(
        pool,
        DeclareShapeParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind,
            kind_owner: None,
            schema,
            enforcement: EnforcementMode::Advisory,
            emitter: w.emitter,
        },
    )
    .await
    .unwrap()
}

/// Store through the substrate write under an explicit family owner.
async fn store_owned(
    pool: &PgPool,
    w: &World,
    kind: &str,
    owner: KindOwner,
    schema: &serde_json::Value,
) -> ShapeId {
    writes::declare_shape(
        pool,
        DeclareShapeParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind,
            kind_owner: Some(owner),
            schema,
            enforcement: EnforcementMode::Advisory,
            emitter: w.emitter,
        },
    )
    .await
    .unwrap()
}

/// A tag no other fixture carries: the root `$comment` `shape_schema::test_support` counts
/// compiles by, so this test's count moves only for this test's schema.
fn fresh_tag() -> String {
    format!("shape-loader-test-{}", Uuid::now_v7())
}

/// A schema that compiles but is past the walk's byte bound: one string property per distinct
/// `pattern`, so a compiler that ran would build thousands of regexes. Tagged with `tag`.
fn patterns_past_the_bound(tag: &str) -> serde_json::Value {
    let properties: serde_json::Map<String, serde_json::Value> = (0..6_000)
        .map(|i| {
            (
                format!("p{i}"),
                json!({ "type": "string", "pattern": format!("^x{i}[a-z]+$") }),
            )
        })
        .collect();
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$comment": tag,
        "type": "object",
        "properties": properties
    });
    assert!(
        !within_schema_bound(&schema),
        "the fixture is past the bound"
    );
    schema
}

/// A schema just within the walk's byte bound: the incumbent value schema, padded with a
/// description to within 64 bytes of the bound.
fn schema_just_within_the_bound() -> serde_json::Value {
    let mut schema = value_schema();
    let base = serde_json::to_vec(&schema).unwrap().len();
    schema["description"] = json!("x".repeat(MAX_SCHEMA_BYTES - base - 64));
    let bytes = serde_json::to_vec(&schema).unwrap().len();
    assert!(
        bytes <= MAX_SCHEMA_BYTES && bytes > MAX_SCHEMA_BYTES - 128,
        "{bytes}"
    );
    assert!(within_schema_bound(&schema));
    schema
}

fn ask_naming(shape: ShapeId) -> ArtifactAsk {
    ArtifactAsk {
        shape,
        predicates: vec![ArtifactPredicate {
            path: vec!["value".to_string()],
            op: PropertyOp::HasKey,
        }],
        intent: vec![],
        include_folded: false,
        shape_states: vec![],
    }
}

/// What one ask naming `shape` is told, through a session over a context loaded for `principal`.
async fn refusals_for(pool: &PgPool, principal: ProfileId, shape: ShapeId) -> Vec<PlanRefusal> {
    let context = load(pool, principal, &BTreeSet::from([shape]))
        .await
        .unwrap();
    context
        .session()
        .validate(&ask_naming(shape))
        .expect_err("the ask is refused")
}

// ── Each arm, from a fixture ─────────────────────────────────────────────────────────────────

/// A live shape the principal can read, whose schema compiles, loads whole: its id and version,
/// its home and family, and its schema, as the gated read returns them. An ask against it
/// validates.
#[sqlx::test(migrations = "../../migrations")]
async fn a_readable_live_compilable_shape_loads(pool: PgPool) {
    let w = seed_world(&pool, "loader-live").await;
    let schema = value_schema();
    let id = declare(&pool, &w, "measurement", &schema).await;
    let row = temper_substrate::readback::shape_by_id(&pool, w.owner, id)
        .await
        .unwrap()
        .expect("the owner reads the shape");

    let ShapeLoad::Loaded(shape) = load_one(&pool, w.owner, id).await.unwrap() else {
        panic!("a readable live compilable shape loads");
    };
    assert_eq!(shape.shape_id, id);
    assert_eq!(shape.shape_version, 1);
    assert_eq!(shape.home_anchor_table, "kb_contexts");
    assert_eq!(shape.home_anchor_id, w.context);
    assert_eq!(shape.kind_owner_table, row.kind_owner.owner_table());
    assert_eq!(shape.kind_owner_id, row.kind_owner.owner_id());
    assert_eq!(shape.artifact_kind, "measurement");
    assert_eq!(shape.schema, schema);

    let context = load(&pool, w.owner, &BTreeSet::from([id])).await.unwrap();
    assert!(context.session().validate(&ask_naming(id)).is_ok());
}

/// A shape the principal cannot read and a shape that does not exist load as the same value, and
/// an ask naming either is told the same thing, byte for byte (spec §3.5). The owner loads the
/// same shape whole, so the stranger's answer is about readability, not absence. Bites a loader
/// that tells the two apart — an ungated probe "for a better message", or a separate arm.
#[sqlx::test(migrations = "../../migrations")]
async fn unreadable_and_absent_shapes_load_identically(pool: PgPool) {
    let w = seed_world(&pool, "loader-unreadable").await;
    let id = declare(&pool, &w, "measurement", &value_schema()).await;
    let absent = ShapeId::from(Uuid::now_v7());

    assert!(
        matches!(
            load_one(&pool, w.owner, id).await.unwrap(),
            ShapeLoad::Loaded(_)
        ),
        "control: the shape exists and its owner can read it"
    );
    let unreadable = load_one(&pool, w.stranger, id).await.unwrap();
    let missing = load_one(&pool, w.owner, absent).await.unwrap();
    assert_eq!(unreadable, ShapeLoad::NotAskable);
    assert_eq!(unreadable, missing);
    assert_eq!(format!("{unreadable:?}"), format!("{missing:?}"));

    let told_unreadable = refusals_for(&pool, w.stranger, id).await;
    let told_absent = refusals_for(&pool, w.owner, absent).await;
    assert_eq!(
        told_unreadable
            .iter()
            .map(|r| r.reason.clone())
            .collect::<Vec<_>>(),
        vec![RefusalReason::ShapeNotAskable]
    );
    assert_eq!(
        serde_json::to_vec(&told_unreadable).unwrap(),
        serde_json::to_vec(&told_absent).unwrap()
    );
}

/// An amended shape's earlier version loads `Superseded`, carrying the version now in force at
/// the same home and family — and not another live row in the same home. Two such rows are stored
/// before the successor, each where a lookup blind to one column finds it first, in insertion
/// order and in the live index's order alike: a sibling family of the same owner whose kind sorts
/// before `measurement`, and the same kind under another owner, whose `kb_profiles` sorts before
/// the family's `kb_teams`. Bites a successor lookup that drops the kind, or the owner, or both,
/// and one that is not made at all.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folded_shape_loads_superseded_with_its_live_successor(pool: PgPool) {
    let w = seed_world(&pool, "loader-folded").await;
    let team: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let family = KindOwner::Team(team);
    let first = store_owned(&pool, &w, "measurement", family, &value_schema()).await;
    store_owned(&pool, &w, "a-sibling", family, &value_schema()).await;
    store_owned(
        &pool,
        &w,
        "measurement",
        KindOwner::Profile(w.owner.uuid()),
        &value_schema(),
    )
    .await;
    let second = store_owned(&pool, &w, "measurement", family, &value_schema()).await;

    assert_eq!(
        load_one(&pool, w.owner, first).await.unwrap(),
        ShapeLoad::Superseded {
            successor: Some(ShapeSuccessor {
                shape_id: second,
                shape_version: 2,
            }),
        }
    );
}

/// The successor lookup carries the anchor gate itself, so it never answers about a home its
/// caller cannot read — whoever calls it, not only the loader, whose step 1 gates first. Bites
/// dropping `anchor_readable_by_profile` from `readback::live_shape_at`.
#[sqlx::test(migrations = "../../migrations")]
async fn the_successor_lookup_answers_only_a_principal_who_can_read_the_home(pool: PgPool) {
    let w = seed_world(&pool, "loader-live-at-gate").await;
    let id = declare(&pool, &w, "measurement", &value_schema()).await;
    let row = readback::shape_by_id(&pool, w.owner, id)
        .await
        .unwrap()
        .expect("the owner reads the shape");

    let for_owner = readback::live_shape_at(
        &pool,
        w.owner,
        row.home_anchor,
        row.kind_owner,
        &row.artifact_kind,
    )
    .await
    .unwrap();
    assert_eq!(for_owner.map(|live| live.shape_id), Some(id));
    let for_stranger = readback::live_shape_at(
        &pool,
        w.stranger,
        row.home_anchor,
        row.kind_owner,
        &row.artifact_kind,
    )
    .await
    .unwrap();
    assert_eq!(for_stranger, None);
}

/// A folded shape with no live row at its home and family loads `Superseded` with no successor,
/// and does not fail. Unreachable through today's writes (amendment is the only fold, and it
/// inserts the successor), so the live row is folded directly. Bites a loader that treats the
/// missing successor as a fault.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folded_shape_with_no_live_successor_loads_superseded_without_one(pool: PgPool) {
    let w = seed_world(&pool, "loader-orphan-fold").await;
    let id = declare(&pool, &w, "measurement", &value_schema()).await;
    sqlx::query("UPDATE kb_data_artifact_shapes SET is_folded = true WHERE id = $1")
        .bind(id.uuid())
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        load_one(&pool, w.owner, id).await.unwrap(),
        ShapeLoad::Superseded { successor: None }
    );
}

/// A stored live shape whose `$ref` leaves the document loads `Uncompilable`, and the listener the
/// `$ref` names sees no connection. Bites a loader that skips the compile, or compiles with
/// default options (with the resolver features re-enabled: see `shape_schema`).
#[sqlx::test(migrations = "../../migrations")]
async fn a_stored_remote_ref_shape_loads_uncompilable_and_fetches_nothing(pool: PgPool) {
    let w = seed_world(&pool, "loader-uncompilable").await;
    let probe = FetchProbe::bind();
    let id = store(&pool, &w, "measurement", &remote_ref_schema(&probe)).await;

    assert_eq!(
        load_one(&pool, w.owner, id).await.unwrap(),
        ShapeLoad::Uncompilable
    );
    assert_eq!(probe.connections(), 0, "the loader must not fetch");
}

// ── The order is fixed ───────────────────────────────────────────────────────────────────────

/// Step 1 precedes 2 and 3: a stranger is told `NotAskable` for a folded shape and for an
/// uncompilable one, never what the row says. Bites a fold or compile check reached by a read that
/// does not pass the gate.
#[sqlx::test(migrations = "../../migrations")]
async fn readability_is_decided_before_fold_and_compile(pool: PgPool) {
    let w = seed_world(&pool, "loader-order-1").await;
    let probe = FetchProbe::bind();
    let folded = declare(&pool, &w, "measurement", &value_schema()).await;
    declare(&pool, &w, "measurement", &value_schema()).await;
    let uncompilable = store(&pool, &w, "reading", &remote_ref_schema(&probe)).await;

    for id in [folded, uncompilable] {
        assert_eq!(
            load_one(&pool, w.stranger, id).await.unwrap(),
            ShapeLoad::NotAskable
        );
    }
    assert_eq!(probe.connections(), 0);
}

/// Step 2 precedes 3: a folded shape whose schema does not compile is `Superseded`, teaching its
/// successor, not `Uncompilable`. Bites the compile run before the fold check.
#[sqlx::test(migrations = "../../migrations")]
async fn fold_is_decided_before_compile(pool: PgPool) {
    let w = seed_world(&pool, "loader-order-2").await;
    let probe = FetchProbe::bind();
    let broken = store(&pool, &w, "measurement", &remote_ref_schema(&probe)).await;
    let mended = store(&pool, &w, "measurement", &value_schema()).await;

    assert_eq!(
        load_one(&pool, w.owner, broken).await.unwrap(),
        ShapeLoad::Superseded {
            successor: Some(ShapeSuccessor {
                shape_id: mended,
                shape_version: 2,
            }),
        }
    );
    assert_eq!(probe.connections(), 0);
}

/// `load` is `load_one` for each id it is given, and nothing for an id it is not.
#[sqlx::test(migrations = "../../migrations")]
async fn load_answers_each_named_shape_as_load_one_does(pool: PgPool) {
    let w = seed_world(&pool, "loader-batch").await;
    let probe = FetchProbe::bind();
    let folded = declare(&pool, &w, "measurement", &value_schema()).await;
    let live = declare(&pool, &w, "measurement", &value_schema()).await;
    let uncompilable = store(&pool, &w, "reading", &remote_ref_schema(&probe)).await;
    let absent = ShapeId::from(Uuid::now_v7());
    let ids = BTreeSet::from([folded, live, uncompilable, absent]);

    let context: ShapeContext = load(&pool, w.owner, &ids).await.unwrap();
    for id in ids {
        assert_eq!(
            context.get(&id),
            Some(&load_one(&pool, w.owner, id).await.unwrap()),
            "{id:?}"
        );
    }
    assert_eq!(context.get(&ShapeId::from(Uuid::now_v7())), None);
}

// ── `askable` on the by-id read ──────────────────────────────────────────────────────────────

/// The by-id read carries the shape's vocabulary; the listing leaves the field absent — not empty,
/// and not on the wire at all. Bites `askable` filled on the listing, or left off the by-id read.
#[sqlx::test(migrations = "../../migrations")]
async fn askable_is_on_the_by_id_read_and_absent_from_the_listing(pool: PgPool) {
    let w = seed_world(&pool, "loader-askable").await;
    let id = declare(&pool, &w, "measurement", &value_schema()).await;

    let shown = substrate_read::get_shape(&pool, w.owner, id)
        .await
        .unwrap()
        .expect("the owner reads the shape");
    let askable = shown
        .askable
        .clone()
        .expect("the by-id read carries askable");
    assert!(askable.truncated.is_empty());
    assert_eq!(askable.paths.len(), 1);
    assert_eq!(askable.paths[0].path, vec!["value".to_string()]);
    assert_eq!(
        askable.paths[0].facts,
        PathFacts {
            types: Some([JsonType::String].into_iter().collect()),
            ..PathFacts::default()
        }
    );
    assert_eq!(shown.unaskable, None);

    let listed = substrate_read::list_shapes(
        &pool,
        w.owner,
        HomeAnchor::Context(ContextId::from(w.context)),
    )
    .await
    .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].askable, None);
    let wire = serde_json::to_value(&listed[0]).unwrap();
    assert!(wire.get("askable").is_none(), "{wire}");
    assert!(wire.get("unaskable").is_none(), "{wire}");
}

/// A stored shape whose schema does not compile reads `askable` empty, with `unaskable` the very
/// refusal an ask naming it meets — reason and fixed sentence — and nothing is fetched; read again,
/// it answers the same. Bites an `askable` derived without the compile (it would read empty with no
/// reason), a reason of its own beside the ask's, or a vocabulary remembered for a shape that does
/// not compile (call `remember_vocabulary` for every verdict in `shape_loader::askable`: the second
/// read answers with no refusal).
#[sqlx::test(migrations = "../../migrations")]
async fn an_uncompilable_shape_reads_askable_empty_with_the_reason(pool: PgPool) {
    let w = seed_world(&pool, "loader-askable-uncompilable").await;
    let probe = FetchProbe::bind();
    let id = store(&pool, &w, "measurement", &remote_ref_schema(&probe)).await;

    let shown = substrate_read::get_shape(&pool, w.owner, id)
        .await
        .unwrap()
        .expect("the owner reads the shape");
    let askable = shown.askable.clone().expect("present, and empty");
    assert!(askable.paths.is_empty());
    assert!(askable.truncated.is_empty());
    let reason = shown
        .unaskable
        .clone()
        .expect("the reason beside the empty vocabulary");
    assert_eq!(reason.reason, RefusalReason::ShapeSchemaUncompilable);
    assert_eq!(
        Some(reason.clone()),
        ShapeLoad::Uncompilable.loaded().err(),
        "the read's reason is the load's refusal"
    );
    assert_eq!(refusals_for(&pool, w.owner, id).await, vec![reason]);

    let wire = serde_json::to_value(&shown).unwrap();
    assert_eq!(wire["askable"], serde_json::json!({ "paths": [] }));
    assert_eq!(wire["unaskable"]["reason"], "shape_schema_uncompilable");

    let again = substrate_read::get_shape(&pool, w.owner, id)
        .await
        .unwrap()
        .expect("the owner reads the shape again");
    assert_eq!(again.askable, shown.askable);
    assert_eq!(again.unaskable, shown.unaskable, "still refused");
    assert_eq!(probe.connections(), 0, "the read must not fetch");
}

// ── The byte bound comes before the compile ──────────────────────────────────────────────────

/// A stored schema past the walk's byte bound — compilable, and costly to compile — is refused at
/// the load and answered by the by-id read with the bound, and is never compiled by either: the
/// load is `SchemaTooLarge`, an ask is told `shape_schema_uncompilable` with
/// `teaches.beyond_bound: schema_bytes`, the read's `askable` is empty and truncated at
/// `schema_bytes` with that same refusal as `unaskable`, and `shape_schema::compile` — counted
/// wherever it is called — never runs on it; read again, it answers the same. Bites removing the
/// measurement from `shape_loader::judge` (the `if !within_schema_bound(schema)` return of
/// `TooLarge`): the schema compiles, the count moves, and it loads `Loaded`. Bites, too, a
/// vocabulary remembered for a shape past the bound: the second read answers with no refusal.
#[sqlx::test(migrations = "../../migrations")]
async fn a_schema_past_the_byte_bound_is_refused_at_load_and_read_and_never_compiled(pool: PgPool) {
    let w = seed_world(&pool, "loader-past-the-bound").await;
    let tag = fresh_tag();
    let id = store(&pool, &w, "measurement", &patterns_past_the_bound(&tag)).await;
    let too_large = ShapeLoad::SchemaTooLarge
        .loaded()
        .expect_err("a schema past the bound is refused");

    assert_eq!(
        load_one(&pool, w.owner, id).await.unwrap(),
        ShapeLoad::SchemaTooLarge
    );
    assert_eq!(
        refusals_for(&pool, w.owner, id).await,
        vec![too_large.clone()]
    );

    let shown = substrate_read::get_shape(&pool, w.owner, id)
        .await
        .unwrap()
        .expect("the owner reads the shape");
    assert_eq!(
        shown.askable,
        Some(Vocabulary {
            paths: vec![],
            truncated: BTreeSet::from([BoundKind::SchemaBytes]),
        })
    );
    assert_eq!(shown.unaskable, Some(too_large));
    let wire = serde_json::to_value(&shown).unwrap();
    assert_eq!(
        wire["askable"],
        json!({ "paths": [], "truncated": ["schema_bytes"] })
    );
    assert_eq!(wire["unaskable"]["reason"], "shape_schema_uncompilable");
    assert_eq!(wire["unaskable"]["teaches"]["beyond_bound"], "schema_bytes");

    let again = substrate_read::get_shape(&pool, w.owner, id)
        .await
        .unwrap()
        .expect("the owner reads the shape again");
    assert_eq!(again.askable, shown.askable);
    assert_eq!(again.unaskable, shown.unaskable, "still refused");

    assert_eq!(
        compiles_tagged(&tag),
        0,
        "nothing compiled a schema past the bound"
    );
}

/// Declare refuses a schema past the walk's byte bound before compiling it, with the typed
/// data-artifact refusal naming the bound and no compiler text, and writes no row; a schema just
/// within the bound still declares. Bites removing the measurement from `shape_loader::judge`: the
/// oversized schema compiles (the count moves) and declares.
#[sqlx::test(migrations = "../../migrations")]
async fn declaring_a_schema_past_the_byte_bound_is_refused_before_it_compiles(pool: PgPool) {
    let w = seed_world(&pool, "declare-past-the-bound").await;
    let tag = fresh_tag();
    let oversized = patterns_past_the_bound(&tag);

    let err = shape_service::declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind: "measurement",
            kind_owner: None,
            schema: &oversized,
            enforcement: EnforcementMode::Advisory,
            principal: w.owner,
            emitter: w.emitter,
        },
    )
    .await
    .expect_err("a schema past the bound is refused at declare");
    let message = match err {
        ApiError::DataArtifactRefusal(message) => message,
        other => panic!("the refusal must be the typed data-artifact refusal, got {other:?}"),
    };
    assert!(
        message.contains(&MAX_SCHEMA_BYTES.to_string()),
        "the refusal names the bound: {message}"
    );
    assert!(
        !message.contains("pattern") && !message.contains("^x"),
        "the refusal carries nothing of the schema: {message}"
    );
    assert_eq!(compiles_tagged(&tag), 0, "declare did not compile it");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_shapes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "a refused declare writes no row");

    let within = declare(&pool, &w, "measurement", &schema_just_within_the_bound()).await;
    assert!(matches!(
        load_one(&pool, w.owner, within).await.unwrap(),
        ShapeLoad::Loaded(_)
    ));
}

// ── A shape read many times is compiled once ────────────────────────────────────────────────

/// Reads of one stored shape — five by-id reads and a load — compile its schema once: the verdict
/// is remembered by shape id after the first. Counted at `shape_schema::compile` by the fixture's
/// tag, so other suites' compiles cannot move it. Bites bypassing the remembered verdict (in
/// `shape_loader::verdict_of`, delete the `if let Some(verdict) = remembered(id)` return): two
/// compiles, the first read's and the load's — the later reads find the vocabulary remembered and
/// ask no verdict.
#[sqlx::test(migrations = "../../migrations")]
async fn a_shape_read_many_times_is_compiled_once(pool: PgPool) {
    let w = seed_world(&pool, "loader-verdict-remembered").await;
    let tag = fresh_tag();
    let mut schema = value_schema();
    schema["$comment"] = json!(tag);
    let id = store(&pool, &w, "measurement", &schema).await;

    for _ in 0..5 {
        let shown = substrate_read::get_shape(&pool, w.owner, id)
            .await
            .unwrap()
            .expect("the owner reads the shape");
        assert!(shown.unaskable.is_none());
    }
    assert!(matches!(
        load_one(&pool, w.owner, id).await.unwrap(),
        ShapeLoad::Loaded(_)
    ));
    assert_eq!(compiles_tagged(&tag), 1, "one compile for six reads");
}

/// Five by-id reads of one stored compiling shape enumerate its vocabulary once: the vocabulary is
/// remembered by shape id after the first, and every read advertises the same one. Counted at
/// `shape_loader::askable`'s enumeration by the fixture's tag, so other suites' reads cannot move
/// it. Bites bypassing the remembered vocabulary (in `shape_loader::askable`, delete the
/// `remembered_vocabulary(row.shape_id)` lookup): five enumerations.
#[sqlx::test(migrations = "../../migrations")]
async fn a_shape_read_many_times_is_enumerated_once(pool: PgPool) {
    let w = seed_world(&pool, "loader-vocabulary-remembered").await;
    let tag = fresh_tag();
    let mut schema = value_schema();
    schema["$comment"] = json!(tag);
    let id = store(&pool, &w, "measurement", &schema).await;

    let mut reads = Vec::new();
    for _ in 0..5 {
        let shown = substrate_read::get_shape(&pool, w.owner, id)
            .await
            .unwrap()
            .expect("the owner reads the shape");
        reads.push(shown.askable.expect("the by-id read carries askable"));
    }
    assert_eq!(reads[0].paths.len(), 1);
    assert_eq!(reads[0].paths[0].path, vec!["value".to_string()]);
    assert!(
        reads.iter().all(|askable| *askable == reads[0]),
        "every read advertises the same vocabulary"
    );
    assert_eq!(
        enumerations_tagged(&tag),
        1,
        "one enumeration for five reads"
    );
}
