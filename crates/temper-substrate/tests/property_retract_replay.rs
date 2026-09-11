#![cfg(feature = "artifact-tests")]
//! Replay stability of `property_retracted` — the row-grain correction's ledger roundtrip.
//!
//! The retraction's payload carries the row id (identity-as-input, the assert payload's own
//! premise), so replay re-folds the SAME row: no duplicate, no resurrection. This is the
//! payload-id-stable scope the retract write is named for — facet-keyed rows are projector-minted
//! surrogates and deliberately unreachable by this verb.
//!
//! ONNX-dependent, like every artifact test. Isolated ephemeral DB via
//! `temper_substrate::MIGRATOR`.

mod common;

use temper_core::types::facet_requests::ANCHORED_AT_PROPERTY_KEY;
use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::events::{EdgeHome, EventContext};
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::replay;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

// Local fixture helpers — duplicated per file rather than shared, the established convention
// (see replay_roundtrip.rs's own header note).

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
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

async fn make_home(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn make_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    title: &str,
    uri: &str,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: uri,
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

/// The full roundtrip: assert an `anchored-at` row, retract it, snapshot, reset to a clean
/// UN-seeded namespace, replay, and prove the SAME row comes back folded — no duplicate row,
/// no resurrection, and `last_event_id` still the retraction event. A missing
/// `EventKind::PropertyRetracted` arm would hard-fail the replay walk here ("no projector for
/// event type property_retracted"), exactly the `resource_finalized` bug shape.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_reprojects_a_property_retraction_onto_the_same_row(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "retract-replay").await;
    let source = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "deriver",
        "temper://retract/deriver",
    )
    .await;
    let target = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "ancestor",
        "temper://retract/ancestor",
    )
    .await;

    let edge = writes::assert_anchored_edge_with(
        &pool,
        writes::AssertAnchoredEdgeParams {
            source: AnchorRef::resource(source),
            target: AnchorRef::resource(target),
            kind: temper_substrate::affinity::EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("derived_from"),
            weight: 1.0,
            home: EdgeHome::Context(home),
            emitter,
        },
        EventContext::default(),
    )
    .await
    .unwrap();

    let address = serde_json::json!({
        "endpoint": "source",
        "address": format!("{source}#{}", Uuid::now_v7()),
    });
    let property_id = writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::Edge { id: edge },
        ANCHORED_AT_PROPERTY_KEY,
        &address,
        1.0,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();

    writes::retract_property_with(&pool, edge, property_id, emitter, EventContext::default())
        .await
        .unwrap();

    // The row is identified by its id — it is payload-id-stable, the whole scope premise; no
    // masked-surrogate identity rules apply here. Capture the tuple BEFORE the reset.
    const BY_ID: &str =
        "SELECT is_folded, last_event_id, property_value, weight, asserted_by_event_id \
         FROM kb_properties WHERE id = $1";
    let before: (bool, Uuid, serde_json::Value, f64, Uuid) = sqlx::query_as(BY_ID)
        .bind(property_id.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(before.0, "the fire path folded the row");

    let retraction_event: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e \
          JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name = 'property_retracted'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        before.1, retraction_event,
        "the fold stamps the retraction event"
    );

    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();

    let after: (bool, Uuid, serde_json::Value, f64, Uuid) = sqlx::query_as(BY_ID)
        .bind(property_id.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        before, after,
        "the same row is re-folded identically — every column"
    );

    let live_or_folded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_edges' AND owner_id = $1 \
            AND property_key = 'anchored-at'",
    )
    .bind(edge.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        live_or_folded, 1,
        "no duplicate — one row per address, folded"
    );

    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_edges' AND owner_id = $1 AND NOT is_folded",
    )
    .bind(edge.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live, 0, "no resurrection — the retracted row stays folded");
}
