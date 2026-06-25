#![cfg(feature = "artifact-tests")]
// Resets the temper_next artifact, then verifies the system boot-seed (event-type registry + global
// lenses via lens_create) lands and is idempotent. Serialized via the temper-substrate-write test-group.
mod common;

use temper_substrate::scenario::bootseed;
use temper_substrate::substrate;

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
        lenses, 4,
        "exactly four global system lenses after (idempotent) seeding (telos-default x2 + orientation + wayfinding)"
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
    assert_eq!(sys_lens_events, 4, "four system-scope lens_created events");
}

#[tokio::test]
async fn bootseed_creates_orientation_and_wayfinding_lenses() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();

    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM kb_cogmap_lenses WHERE cogmap_id IS NULL ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(
        names.contains(&"orientation".to_string()),
        "expected a global `orientation` posture-lens, got {names:?}"
    );
    assert!(
        names.contains(&"wayfinding".to_string()),
        "expected a global `wayfinding` posture-lens, got {names:?}"
    );
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
        temper_substrate::payloads::TYPED_EVENT_NAMES.len() as i64,
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

/// Drift guard: the production system seed (`migrations/…_canonical_seed.sql`) and the test
/// boot-seed vocabulary (`tests/fixtures/seeds/system.yaml`) both encode the event-type registry.
/// They are separate sources (production SQL vs the test bootseed YAML), so this asserts every
/// system.yaml event-type name appears in the seed migration's `kb_event_types` INSERT — they can
/// never silently drift. Replaces the retired schema_drift.rs two-copy guard. No DB needed.
#[test]
fn seed_migration_event_types_match_system_yaml() {
    let yaml_names =
        bootseed::system_event_type_names().expect("read system.yaml event-type names");
    let migration = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20260624000003_canonical_seed.sql"
    ))
    .expect("read canonical seed migration");
    for name in &yaml_names {
        assert!(
            migration.contains(&format!("('{name}',")),
            "event type `{name}` is in system.yaml but missing from the canonical seed migration"
        );
    }
}
