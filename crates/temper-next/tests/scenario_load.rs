#![cfg(feature = "artifact-tests")]
//! Loader integration test: boot-seed + load a minimal seed, then confirm `substrate::load` reads
//! the homed nodes + edges back. Resets the artifact first (owns the namespace; serialized).
mod common;

use temper_next::scenario::{loader, model::Seed};
use temper_next::substrate;

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

#[tokio::test]
async fn loads_minimal_seed_into_readable_substrate() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    temper_next::scenario::bootseed::seed_system(&pool)
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
#[tokio::test]
async fn resource_key_colliding_with_reserved_telos_is_rejected() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    temper_next::scenario::bootseed::seed_system(&pool)
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
