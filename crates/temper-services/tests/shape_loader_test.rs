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
//! `shape_schema_no_fetch` tests in temper-substrate, whose constructor the loader compiles through.
#![cfg(feature = "test-db")]

use std::collections::BTreeSet;

use sqlx::PgPool;
use temper_core::types::artifact_ask::{
    ArtifactAsk, ArtifactPredicate, JsonType, PathFacts, ShapeContext, ShapeLoad, ShapeSuccessor,
};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EntityId, ProfileId, ShapeId};
use temper_core::types::query::{PlanRefusal, PropertyOp, RefusalReason};
use temper_services::backend::substrate_read;
use temper_services::services::shape_loader::{load, load_one};
use temper_services::services::shape_service::{self, DeclareShapeServiceParams};
use temper_substrate::events::EventContext;
use temper_substrate::payloads::{AnchorRef, EnforcementMode};
use temper_substrate::scenario::bootseed;
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
/// the same home and family. Bites a successor looked up by anything other than the home and
/// family, or not looked up at all.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folded_shape_loads_superseded_with_its_live_successor(pool: PgPool) {
    let w = seed_world(&pool, "loader-folded").await;
    let first = declare(&pool, &w, "measurement", &value_schema()).await;
    // A sibling family in the same home: never the successor.
    declare(&pool, &w, "reading", &value_schema()).await;
    let second = declare(&pool, &w, "measurement", &value_schema()).await;

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
/// refusal an ask naming it meets — reason and fixed sentence — and nothing is fetched. Bites an
/// `askable` derived without the compile (it would read empty with no reason), or a reason of its
/// own beside the ask's.
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
    assert_eq!(probe.connections(), 0, "the read must not fetch");
}
