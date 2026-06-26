#![cfg(feature = "artifact-tests")]
//! Loader integration test: boot-seed + load a minimal seed, then confirm `substrate::load` reads
//! the homed nodes + edges back. Isolated ephemeral DB via `temper_substrate::MIGRATOR`.
mod common;

use temper_substrate::scenario::{loader, model::Seed};
use temper_substrate::substrate;

const MINIMAL: &str = r#"
name: minimal-load-test
cogmap:
  telos: { title: "Min", statement: "A tiny telos.", questions: [{ question: "why?" }] }
  owner: alice
  emitter: "agent#1"
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: "agent#1", profile: alice }]
resources:
  - { key: a, origin_uri: "temper://min/a", home: cogmap, body: "alpha body", facets: { values: { phase: x } } }
  - { key: b, origin_uri: "temper://min/b", home: cogmap, body: "beta body" }
edges:
  - { from: a, to: b, kind: leads_to, label: then, weight: 1.0 }
  - { from: telos, to: a, kind: express, label: operationalized_by }
uses_lenses: [telos-default]
"#;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn loads_minimal_seed_into_readable_substrate(pool: sqlx::PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();

    let s: Seed = serde_yaml::from_str(MINIMAL).unwrap();
    let loaded = loader::load_seed(&pool, &s).await.unwrap();

    // the implicit telos key resolves, and edges can reference it
    assert!(loaded.keys.contains_key("telos"));
    assert!(loaded.keys.contains_key("a"));

    // substrate::load sees the homed nodes (telos + a + b) and both declared edges, against the
    // boot-seeded global telos-default lens.
    let sub = substrate::load(&pool, loaded.cogmap, "telos-default")
        .await
        .unwrap();
    assert_eq!(sub.nodes.len(), 3, "telos + a + b are homed");
    assert_eq!(sub.edges.len(), 2, "leads_to(a->b) + express(telos->a)");
    // the facet on `a` expanded to one Facet entry
    assert_eq!(sub.facets.len(), 1, "one facet on resource a");
}

/// Guard: a resource keyed `telos` would silently shadow the implicit charter key and corrupt the
/// charter read. The loader rejects it fast with a clear collision error.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn resource_key_colliding_with_reserved_telos_is_rejected(pool: sqlx::PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();

    const COLLIDING: &str = r#"
name: collision-test
cogmap:
  telos: { title: "T", statement: "S", questions: [{ question: "q?" }] }
  owner: alice
  emitter: "agent#1"
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: "agent#1", profile: alice }]
resources:
  - { key: telos, origin_uri: "temper://c/telos", home: cogmap, body: "a concept wrongly reusing the reserved key" }
uses_lenses: [telos-default]
"#;
    let s: Seed = serde_yaml::from_str(COLLIDING).unwrap();
    let err = loader::load_seed(&pool, &s)
        .await
        .err()
        .expect("expected load_seed to fail on reserved-key collision");
    assert!(
        err.to_string().contains("collides with an existing key"),
        "expected reserved-key collision error, got: {err}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn lens_name_parameter_binds_the_lens_query(pool: sqlx::PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool).await.unwrap();
    let s: Seed = serde_yaml::from_str(MINIMAL).unwrap();
    let loaded = loader::load_seed(&pool, &s).await.unwrap();
    substrate::load(&pool, loaded.cogmap, "telos-default")
        .await
        .expect("telos-default lens loads by name");
    let bogus = substrate::load(&pool, loaded.cogmap, "no-such-lens").await;
    assert!(bogus.is_err(), "loading an unknown lens name must error");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn seeded_telos_default_lens_mirrors_the_rust_default(pool: sqlx::PgPool) {
    use temper_substrate::affinity::Lens;
    temper_substrate::scenario::bootseed::seed_system(&pool).await.unwrap();
    let s: Seed = serde_yaml::from_str(MINIMAL).unwrap();
    let loaded = loader::load_seed(&pool, &s).await.unwrap();
    let sub = substrate::load(&pool, loaded.cogmap, "telos-default").await.unwrap();
    let d = Lens::telos_default();
    assert_eq!(sub.lens.w_express, d.w_express, "w_express");
    assert_eq!(sub.lens.w_contains, d.w_contains, "w_contains");
    assert_eq!(sub.lens.w_leads_to, d.w_leads_to, "w_leads_to");
    assert_eq!(sub.lens.w_near, d.w_near, "w_near");
    assert_eq!(sub.lens.w_prop, d.w_prop, "w_prop");
    assert_eq!(sub.lens.s_telos, d.s_telos, "s_telos");
    assert_eq!(sub.lens.s_ref, d.s_ref, "s_ref");
    assert_eq!(sub.lens.s_central, d.s_central, "s_central");
    assert_eq!(sub.lens.resolution, d.resolution, "resolution");
}
