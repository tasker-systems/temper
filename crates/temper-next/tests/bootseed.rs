#![cfg(feature = "artifact-tests")]
// Resets the temper_next artifact, then verifies the system boot-seed (event-type registry + global
// lenses via lens_create) lands and is idempotent. Serialized via the temper-next-write test-group.
mod common;

use temper_next::scenario::bootseed;
use temper_next::substrate;

#[tokio::test]
async fn seed_system_seeds_registry_and_global_lenses_idempotently() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    bootseed::seed_system(&pool).await.unwrap(); // idempotent — second run is a no-op

    let lenses: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_cogmap_lenses WHERE cogmap_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        lenses, 2,
        "exactly two global system lenses after (idempotent) seeding"
    );

    let has_lens_created: bool =
        sqlx::query_scalar("SELECT exists(SELECT 1 FROM kb_event_types WHERE name='lens_created')")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(has_lens_created, "lens_created event type registered");

    // the system actor emitted the lens_created events with a both-null (system-scope) anchor
    let sys_lens_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id=e.event_type_id \
         WHERE et.name='lens_created' AND e.producing_anchor_table IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sys_lens_events, 2, "two system-scope lens_created events");
}

#[tokio::test]
async fn bootseed_publishes_payload_schemas() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();

    // exactly the 15 typed names carry a published schema (registry == committed snapshots)
    let stamped: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_event_types WHERE payload_schema IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        stamped,
        temper_next::payloads::TYPED_EVENT_NAMES.len() as i64,
        "exactly the typed names carry a published schema"
    );

    // an untyped name stays NULL = unregistered/permissive (the foreign-event posture)
    let permissive: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload_schema FROM kb_event_types WHERE name = 'delegated_launch'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        permissive.is_none(),
        "untyped names stay NULL (unregistered/permissive)"
    );
}
