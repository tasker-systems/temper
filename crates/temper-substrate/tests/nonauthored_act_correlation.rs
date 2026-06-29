#![cfg(feature = "artifact-tests")]
//! Chunk 1 stamp proof (task 019f10c5): the **non-authored** write arms thread `EventContext` into
//! `kb_events.invocation_id` (dedicated nullable column) + `kb_events.metadata` (authorship), exactly
//! like the authored-4. Before this task these arms called their SQL fns with `($1,$2)` and silently
//! dropped `ctx` — so an `update`/`property_set` under a run never appeared in `invocation_show`.
//!
//! The binding is uniform across every non-authored arm (each forwards `ctx_meta`/`ctx_inv` into the
//! shared `_event_append` sink), and `cargo make prepare-next` fails the build if any arm's arg count
//! drifts from its SQL fn — so this proves the end-to-end column write on representative resource-shaped
//! arms (`resource_update`, `property_set`); the edge/block/charter arms are exercised at the backend +
//! e2e layers. Also proves the default `fire()` path is byte-identical (NULL invocation, `{}` metadata).

use serde_json::json;
use sqlx::PgPool;
use temper_substrate::events::{fire, fire_with, EventContext, SeedAction};
use temper_substrate::ids::{CogmapId, EntityId, ResourceId};
use temper_substrate::payloads::{AgentAuthorship, ConfidenceBand};
use temper_substrate::writes::{self, OpenParams};
use uuid::Uuid;

mod common;

/// The boot-seeded canonical `system` entity id (emitter for the fires).
async fn system_entity(pool: &PgPool) -> Uuid {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
        .bind(profile)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// `(invocation_id, metadata)` of the most-recent event of a given type (uuid-v7 id breaks occurred_at
/// ties — both are time-ordered).
async fn latest_event(pool: &PgPool, type_name: &str) -> (Option<Uuid>, serde_json::Value) {
    sqlx::query_as(
        "SELECT e.invocation_id, e.metadata \
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = $1 ORDER BY e.occurred_at DESC, e.id DESC LIMIT 1",
    )
    .bind(type_name)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("no `{type_name}` event found: {e}"))
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn nonauthored_arms_stamp_invocation_and_metadata(pool: PgPool) {
    common::seed_system(&pool).await;
    let (cogmap, telos) = common::genesis_cogmap(&pool, "nonauth-test", "Non-Authored Test").await;
    let entity = EntityId::from(system_entity(&pool).await);

    let inv = writes::open_invocation(
        &pool,
        OpenParams {
            trigger_kind: "agent_run".to_string(),
            originating: CogmapId::from(cogmap),
            parent: None,
            scoped_entity: entity,
            emitter: entity,
        },
    )
    .await
    .expect("open invocation");

    let authorship = AgentAuthorship {
        reasoning: Some("retitled per the new charter".to_string()),
        confidence: ConfidenceBand::Confident,
        rationale: None,
        persona: None,
        model: None,
    };
    let stamped = || EventContext {
        authorship: Some(authorship.clone()),
        invocation: Some(inv),
    };

    // STAMPED resource_update — the headline non-authored act.
    let mut tx = pool.begin().await.unwrap();
    fire_with(
        &mut tx,
        SeedAction::ResourceUpdate {
            resource: ResourceId::from(telos),
            title: Some("New Telos Title"),
            origin_uri: None,
            emitter: entity,
        },
        stamped(),
    )
    .await
    .expect("stamped resource_update");
    tx.commit().await.unwrap();

    let (inv_id, meta) = latest_event(&pool, "resource_updated").await;
    assert_eq!(
        inv_id,
        Some(inv.uuid()),
        "resource_updated carries invocation_id"
    );
    assert_eq!(
        meta["confidence"],
        json!("confident"),
        "metadata carries authorship: {meta}"
    );

    // STAMPED property_set — a sub-event of the update fan-out (also fired standalone by the reconciler).
    let val = json!("kernel");
    let mut tx = pool.begin().await.unwrap();
    fire_with(
        &mut tx,
        SeedAction::PropertySet {
            resource: ResourceId::from(telos),
            key: "provenance",
            value: &val,
            weight: 1.0,
            emitter: entity,
        },
        stamped(),
    )
    .await
    .expect("stamped property_set");
    tx.commit().await.unwrap();

    let (inv_id, meta) = latest_event(&pool, "property_set").await;
    assert_eq!(
        inv_id,
        Some(inv.uuid()),
        "property_set carries invocation_id"
    );
    assert_eq!(
        meta["reasoning"],
        json!("retitled per the new charter"),
        "metadata carries authorship reasoning: {meta}"
    );

    // DEFAULT path (regression): a plain `fire()` leaves invocation_id NULL + metadata '{}' — byte-identical
    // to pre-task behavior.
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceUpdate {
            resource: ResourceId::from(telos),
            title: Some("Default-Path Title"),
            origin_uri: None,
            emitter: entity,
        },
    )
    .await
    .expect("default resource_update");
    tx.commit().await.unwrap();

    let (inv_id, meta) = latest_event(&pool, "resource_updated").await;
    assert_eq!(inv_id, None, "default fire() leaves invocation_id NULL");
    assert_eq!(meta, json!({}), "default fire() leaves metadata empty");
}
