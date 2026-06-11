#![cfg(feature = "artifact-tests")]
//! Access-scaffold proof: loads the epd-bridge access world from YAML and asserts the kernel gate
//! functions (S1-S5) declaratively, plus the S8 capability-coherence CHECK. These OWN the
//! `temper_next` namespace (each resets it to a clean 01+02 then loads) — serialized via the
//! `temper-next-write` nextest group, ONNX-dependent (the onboarding charter embeds inline).
mod common;

use temper_next::scenario::access::{self, model::AccessScenario};
use temper_next::scenario::bootseed;
use temper_next::substrate;

const ACCESS_SCENARIO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schema-artifact/access-scenarios/epd-bridge-access.yaml"
);

fn load_access_yaml() -> AccessScenario {
    serde_yaml::from_str(&std::fs::read_to_string(ACCESS_SCENARIO).unwrap()).unwrap()
}

#[tokio::test]
async fn loads_topology_row_counts() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();

    let doc = load_access_yaml();
    let loaded = access::load(&pool, &doc.world).await.unwrap();

    assert_eq!(loaded.profiles.len(), 6);
    assert_eq!(loaded.teams.len(), 6);
    assert_eq!(loaded.cogmaps.len(), 5);
    assert_eq!(loaded.resources.len(), 5);

    // Row-count sanity against the DB (bootseed adds no teams).
    let teams: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_teams")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(teams, 6);
    // alice was auto-joined to temper-system root (approved) + joined epd-team-a => 2 memberships.
    let alice_teams: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members m JOIN kb_profiles p ON p.id=m.profile_id WHERE p.handle='alice'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(alice_teams, 2);
    // nomad (system_access=none) joined nothing.
    let nomad_teams: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members m JOIN kb_profiles p ON p.id=m.profile_id WHERE p.handle='nomad'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nomad_teams, 0);
}

#[tokio::test]
async fn proves_all_access_invariants() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();

    access::run_access_scenario(&pool, &load_access_yaml())
        .await
        .expect("all S1-S5 access checks pass");
}
