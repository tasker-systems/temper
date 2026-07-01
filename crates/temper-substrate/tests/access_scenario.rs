#![cfg(feature = "artifact-tests")]
//! Access-scaffold proof: loads the epd-bridge access world from YAML and asserts the kernel gate
//! functions (S1-S5) declaratively, plus the S8 capability-coherence CHECK. Each test runs on an
//! ephemeral `public`-schema database via `#[sqlx::test]`. ONNX-dependent (the onboarding charter
//! embeds inline).
mod common;

use temper_substrate::scenario::access::{self, model::AccessScenario};
use temper_substrate::scenario::bootseed;

const ACCESS_SCENARIO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/access-scenarios/epd-bridge-access.yaml"
);

fn load_access_yaml() -> AccessScenario {
    serde_yaml::from_str(&std::fs::read_to_string(ACCESS_SCENARIO).unwrap()).unwrap()
}

const CONTEXT_SHARE_SCENARIO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/access-scenarios/context-share-access.yaml"
);

fn load_context_share_yaml() -> AccessScenario {
    serde_yaml::from_str(&std::fs::read_to_string(CONTEXT_SHARE_SCENARIO).unwrap()).unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn loads_topology_row_counts(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();

    let doc = load_access_yaml();
    let loaded = access::load(&pool, &doc.world).await.unwrap();

    assert_eq!(loaded.profiles.len(), 6);
    // 6 declared + the loader's DB refresh picks up the 7 trigger-created personal
    // teams (6 fixture profiles + bootseed's `system` profile).
    assert_eq!(loaded.teams.len(), 13);
    assert_eq!(loaded.cogmaps.len(), 5);
    assert_eq!(loaded.resources.len(), 5);

    // Row-count sanity against the DB: 6 declared + one trigger-created personal
    // team per profile (6 fixture profiles + bootseed's `system` profile) = 13.
    let teams: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_teams")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(teams, 13);
    // alice: temper-system root (approved) + epd-team-a + personal-alice => 3 memberships.
    let alice_teams: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members m JOIN kb_profiles p ON p.id=m.profile_id WHERE p.handle='alice'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(alice_teams, 3);
    // nomad (system_access=none) gets ONLY the personal team.
    let nomad_teams: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members m JOIN kb_profiles p ON p.id=m.profile_id WHERE p.handle='nomad'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nomad_teams, 1);
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn proves_all_access_invariants(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();

    access::run_access_scenario(&pool, &load_access_yaml())
        .await
        .expect("all S1-S5 access checks pass");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn proves_context_share_invariants(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();

    access::run_access_scenario(&pool, &load_context_share_yaml())
        .await
        .expect("all context-share leak-safety checks pass");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn s8_capability_check_rejects_write_without_read(pool: sqlx::PgPool) {
    // Minimal anchors. 'none' avoids the root-join trigger.
    let pid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name, system_access) \
         VALUES ('s8user','S8','none') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let rid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('s8','temper://s8') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // can_write=true with can_read=false must be rejected by the coherence CHECK.
    let res = sqlx::query(
        "INSERT INTO kb_access_grants \
         (subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_resources',$1,'kb_profiles',$2,false,true,$2)",
    )
    .bind(rid)
    .bind(pid)
    .execute(&pool)
    .await;

    let err = res.expect_err("write-without-read grant must be rejected");
    let is_check_violation = matches!(
        &err,
        sqlx::Error::Database(e) if e.code().as_deref() == Some("23514")
    );
    assert!(
        is_check_violation,
        "expected check_violation (23514), got {err:?}"
    );
}
