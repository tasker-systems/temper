#![cfg(feature = "test-db")]
//! An update reports the home a move left from the write itself (goal clause
//! `a-write-learns-its-outcome-from-the-write`).
//!
//! `update_resource_in_tx` answers an [`UpdateOutcome`] whose `left_home` is the home a move left,
//! so no caller reads its own `resource_departed` back from the ledger to learn it. Each witness
//! pins the outcome against the departure the same transaction wrote: `left_home` is `Some(home)`
//! exactly when a departure anchored on `home` was appended, and `None` exactly when none was.

mod common;

use sha2::Digest;
use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams, UpdateOutcome, UpdateParams};
use uuid::Uuid;

fn chunk(prose: &str) -> IncomingChunk {
    IncomingChunk {
        chunk_index: 0,
        content_hash: hex::encode(sha2::Sha256::digest(prose.trim())),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: String::new(),
        heading_depth: 0,
    }
}

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

struct Seeded {
    resource: ResourceId,
    emitter: EntityId,
    origin: ContextId,
    elsewhere: ContextId,
}

/// One resource homed in `origin`, and a second context to move it to.
async fn seed(pool: &PgPool) -> Seeded {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let context = |slug: &'static str| async move {
        ContextId::from(
            common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
                .await
                .unwrap(),
        )
    };
    let origin = context("origin").await;
    let elsewhere = context("elsewhere").await;
    let resource = writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: "mover",
            origin_uri: "test://update-outcome",
            body: "the body",
            doc_type: "research",
            home: AnchorRef::context(origin),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk("the body")]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("seed resource through the create path");
    Seeded {
        resource,
        emitter,
        origin,
        elsewhere,
    }
}

/// Run one update in its own transaction and answer its outcome together with the anchors of every
/// `resource_departed` that transaction appended for the resource.
async fn update(
    pool: &PgPool,
    s: &Seeded,
    rehome_to: Option<ContextId>,
    title: Option<&str>,
) -> (UpdateOutcome, Vec<HomeAnchor>) {
    let mut tx = pool.begin().await.unwrap();
    let outcome = writes::update_resource_in_tx(
        &mut tx,
        UpdateParams {
            resource: s.resource,
            body: None,
            title,
            origin_uri: None,
            properties: &[],
            unset_keys: &[],
            chunks: None,
            sources: vec![],
            content_block: None,
            rehome_to,
            emitter: s.emitter,
        },
        EventContext::default(),
        false,
    )
    .await
    .expect("update");
    let departures: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT e.producing_anchor_table, e.producing_anchor_id
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id
          WHERE et.name = 'resource_departed' AND e.xact_id = pg_current_xact_id()
            AND e.payload->>'resource_id' = $1::text",
    )
    .bind(s.resource.uuid())
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let departures = departures
        .into_iter()
        .map(|(t, id)| HomeAnchor::from_parts(&t, id).expect("a home anchor table"))
        .collect();
    (outcome, departures)
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_move_reports_the_home_it_left(pool: PgPool) {
    let s = seed(&pool).await;
    let (outcome, departures) = update(&pool, &s, Some(s.elsewhere), Some("moved")).await;
    let origin = HomeAnchor::Context(s.origin.uuid().into());
    assert_eq!(outcome.left_home, Some(origin));
    assert_eq!(
        departures,
        vec![origin],
        "the outcome names the departure's home"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_move_to_the_home_it_has_reports_no_departure(pool: PgPool) {
    let s = seed(&pool).await;
    let (outcome, departures) = update(&pool, &s, Some(s.origin), None).await;
    assert_eq!(outcome.left_home, None);
    assert!(
        departures.is_empty(),
        "no departure was written: {departures:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_update_without_a_move_reports_no_departure(pool: PgPool) {
    let s = seed(&pool).await;
    let (outcome, departures) = update(&pool, &s, None, Some("retitled")).await;
    assert_eq!(outcome, UpdateOutcome::default());
    assert!(
        departures.is_empty(),
        "no departure was written: {departures:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_second_move_reports_the_home_the_first_move_reached(pool: PgPool) {
    let s = seed(&pool).await;
    let _ = update(&pool, &s, Some(s.elsewhere), None).await;
    let (outcome, departures) = update(&pool, &s, Some(s.origin), None).await;
    let elsewhere = HomeAnchor::Context(s.elsewhere.uuid().into());
    assert_eq!(outcome.left_home, Some(elsewhere));
    assert_eq!(departures, vec![elsewhere]);
}
