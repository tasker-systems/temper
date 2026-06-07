#![cfg(feature = "artifact-tests")]
//! Loader integration test: boot-seed + load a minimal scenario, then confirm `substrate::load` reads
//! the homed nodes + edges back. Resets the artifact first (owns the namespace; serialized).
mod common;

use temper_next::scenario::{loader, model::Scenario};
use temper_next::substrate;

const MINIMAL: &str = r#"
name: minimal-load-test
cogmap:
  telos: { title: "Min", statement: "A tiny telos.", questions: ["why?"] }
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
steps: []
"#;

#[tokio::test]
async fn loads_minimal_scenario_into_readable_substrate() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    temper_next::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();

    let s: Scenario = serde_yaml::from_str(MINIMAL).unwrap();
    let loaded = loader::load_scenario(&pool, &s).await.unwrap();

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
