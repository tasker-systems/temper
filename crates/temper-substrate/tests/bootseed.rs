#![cfg(feature = "artifact-tests")]
// Verifies the system boot-seed (event-type registry + global lenses via lens_create) lands and is
// idempotent on a fresh ephemeral database provisioned by #[sqlx::test].
mod common;

use temper_substrate::scenario::bootseed;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn seed_system_seeds_registry_and_global_lenses_idempotently(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    bootseed::seed_system(&pool).await.unwrap(); // idempotent — second run is a no-op

    let lenses: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_cogmap_lenses WHERE cogmap_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        lenses, 5,
        "exactly five global system lenses after (idempotent) seeding (telos-default x2 + orientation \
         + wayfinding + workflow-default, the context regime's kernel lens)"
    );

    // The context lens is the ONLY one with the kernel switched on. If a cogmap lens ever picks up a
    // nonzero w_cos, formation changes for every map in production — this is the guard on that.
    let regime: Vec<(String, f64)> = sqlx::query_as(
        "SELECT name, w_cos FROM kb_cogmap_lenses WHERE cogmap_id IS NULL ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (name, w_cos) in &regime {
        let expected = if name == "workflow-default" { 1.0 } else { 0.0 };
        assert_eq!(
            *w_cos, expected,
            "lens `{name}` must carry w_cos={expected} — the kernel is on for contexts ONLY"
        );
    }

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
    assert_eq!(
        sys_lens_events, 5,
        "five system-scope lens_created events — workflow-default is event-sourced through \
         lens_create like every other lens, not raw-INSERTed"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn bootseed_creates_orientation_and_wayfinding_lenses(pool: sqlx::PgPool) {
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

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn bootseed_publishes_payload_schemas(pool: sqlx::PgPool) {
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

/// Drift guard: the production migrations and the test boot-seed vocabulary
/// (`tests/fixtures/seeds/system.yaml`) both encode the event-type registry. They are separate
/// sources (production SQL vs the test bootseed YAML), so this asserts every system.yaml event-type
/// name is registered by SOME migration's `kb_event_types` INSERT — they can never silently drift.
/// Replaces the retired schema_drift.rs two-copy guard. No DB needed.
///
/// Scans the WHOLE `migrations/` set, not just the canonical seed: the registry grows by additive
/// migrations (e.g. the principal-admission events in `20260720000020`, a later migration than the
/// checksum-locked canonical seed). Pinning to one historical file would forbid system.yaml from
/// naming any event type added after it — while `seed_system`'s own genesis door emits one.
#[test]
fn seed_migration_event_types_match_system_yaml() {
    let yaml_names =
        bootseed::system_event_type_names().expect("read system.yaml event-type names");
    let migrations_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");
    let mut all_sql = String::new();
    for entry in std::fs::read_dir(migrations_dir).expect("read migrations dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("sql") {
            all_sql.push_str(&std::fs::read_to_string(&path).expect("read migration"));
            all_sql.push('\n');
        }
    }
    for name in &yaml_names {
        assert!(
            all_sql.contains(&format!("('{name}',")),
            "event type `{name}` is in system.yaml but no migration registers it in kb_event_types"
        );
    }
}
