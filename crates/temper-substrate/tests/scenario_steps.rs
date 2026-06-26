#![cfg(feature = "artifact-tests")]
//! D1 acceptance: the new step vocabulary (create_resource / set_facet / assert_edge / fold_edge)
//! drives a real mutation runbook end-to-end. A fold + a competing edge demonstrably change region
//! membership across two materializes — the substrate drift detection (WS5) will later consume.
//! Isolated ephemeral DB via `temper_substrate::MIGRATOR`.
mod common;

use temper_substrate::scenario::{bootseed, model::Scenario, runner};

const SCENARIO: &str = r#"
name: steps-acceptance
seed:
  name: steps-seed
  cogmap:
    telos: { title: T, statement: "A small map for exercising step mutations.", questions: [{ question: "What groups?" }] }
    owner: pete
    emitter: "agent#1"
  world:
    profiles: [{ handle: pete, display_name: Pete, system_access: approved }]
    entities: [{ name: "agent#1", profile: pete }]
  resources: []
  uses_lenses: [telos-default]
steps:
  - { do: create_resource, key: alpha, origin_uri: "temper://c/alpha", body: "deployment pipeline staging and rollout cadence" }
  - { do: create_resource, key: beta,  origin_uri: "temper://c/beta",  body: "deployment pipeline staging and rollout cadence, closely related" }
  - { do: create_resource, key: gamma, origin_uri: "temper://c/gamma", body: "an unrelated note about tea brewing temperature" }
  - { do: assert_edge, from: alpha, to: beta, kind: express, label: related }
  - { do: materialize, lens: telos-default }
  - { do: assert, checks: [{ check: co_region, lens: telos-default, members: [alpha, beta], expect: true }] }
  - { do: assert, checks: [{ check: stale, expect: false }] }
  - { do: fold_edge, from: alpha, to: beta, kind: express, reason: "the bond is retired" }
  - { do: assert, checks: [{ check: stale, expect: true }] }
  - { do: assert_edge, from: alpha, to: gamma, kind: express, label: related }
  - { do: materialize, lens: telos-default }
  - { do: assert, checks: [{ check: co_region, lens: telos-default, members: [alpha, gamma], expect: true }] }
"#;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn step_vocabulary_drives_a_mutation_runbook(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let scenario: Scenario = serde_yaml::from_str(SCENARIO).unwrap();
    runner::run_scenario(&pool, &scenario, std::path::Path::new("."))
        .await
        .expect("inline step runbook passes its declarative asserts");

    // every fired event (incl. relationship_folded) deserializes into its typed payload struct
    temper_substrate::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger payload roundtrip incl. fold");
}
