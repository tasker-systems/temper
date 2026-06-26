#![cfg(feature = "artifact-tests")]
//! Reconcile diff-read: `kernel_slice` returns only `provenance: kernel` resources homed to the
//! cogmap, keyed by `origin_uri`, carrying `body_hash` and the merged facet object. Runs against an
//! isolated `#[sqlx::test]` ephemeral DB (MIGRATOR-applied canonical schema + seed).
//!
//! Provenance is a per-KEY frontmatter property (`property_key='provenance'`), the same per-key shape
//! `resource_row`/`meta` read (and the inverse of WS6 §7's one-row-per-key synthesis) — NOT the
//! scenario-DSL `facets:` map (which `FacetSet` folds into a single `property_key='facet'` object for
//! region clustering). So the seed DSL stands up the cogmap + bodies, and the provenance/layer markers
//! are fired as per-key `PropertyAssert`s, exactly as the reconcile applier will write them.

use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{EntityId, ResourceId};
use temper_substrate::scenario::{loader, model::Seed};

const SEED: &str = r#"
name: kernel-slice-test
cogmap:
  telos: { title: "K", statement: "Kernel telos.", questions: [{ question: "why?" }] }
  owner: alice
  emitter: "agent#1"
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: "agent#1", profile: alice }]
resources:
  - { key: a, origin_uri: "temper://kernel/concept/cogmap", home: cogmap, body: "alpha kernel body" }
  - { key: b, origin_uri: "temper://kernel/promoted/thing", home: cogmap, body: "promoted body" }
uses_lenses: [telos-default]
"#;

async fn assert_property(
    pool: &sqlx::PgPool,
    resource: ResourceId,
    emitter: EntityId,
    key: &str,
    value: &str,
) {
    let json = serde_json::Value::String(value.to_owned());
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::PropertyAssert {
            resource,
            key,
            value: &json,
            weight: 1.0,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn kernel_slice_returns_only_kernel_provenance(pool: sqlx::PgPool) {
    // The MIGRATOR ephemeral DB already carries the canonical seed (the `system` profile/entity); the
    // scenario seed below stands up the cogmap + bodies.
    let s: Seed = serde_yaml::from_str(SEED).unwrap();
    let loaded = loader::load_seed(&pool, &s).await.unwrap();

    // The boot-seeded canonical system entity is the property emitter (the home anchors the event).
    let profile: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let entity: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(&pool)
            .await
            .unwrap();
    let emitter = EntityId::from(entity);

    let a = ResourceId::from(loaded.keys["a"]);
    let b = ResourceId::from(loaded.keys["b"]);
    assert_property(&pool, a, emitter, "provenance", "kernel").await;
    assert_property(&pool, a, emitter, "layer", "concept").await;
    assert_property(&pool, b, emitter, "provenance", "promoted").await;

    let rows = temper_substrate::readback::kernel_slice(&pool, loaded.cogmap)
        .await
        .unwrap();

    // Exactly the one kernel resource — the promoted resource and the (provenance-less) telos are
    // excluded.
    assert_eq!(
        rows.len(),
        1,
        "only the provenance:kernel resource is in the slice"
    );
    let uris: Vec<_> = rows.iter().map(|r| r.origin_uri.as_str()).collect();
    assert!(uris.contains(&"temper://kernel/concept/cogmap"));
    assert!(
        !uris.iter().any(|u| u.contains("/promoted/")),
        "promoted-provenance resource must be excluded"
    );

    // Every kernel row carries a body merkle (the diff signal) and its merged facet object.
    assert!(
        rows.iter().all(|r| r.body_hash.is_some()),
        "body_hash populated"
    );
    let facets = &rows[0].facets;
    assert_eq!(
        facets.get("provenance").and_then(|v| v.as_str()),
        Some("kernel")
    );
    assert_eq!(
        facets.get("layer").and_then(|v| v.as_str()),
        Some("concept")
    );
}
