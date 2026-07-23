#![cfg(feature = "test-db")]

mod common;

use temper_core::types::admin::UpdateSettingsRequest;
use temper_services::services::access_service;
use temper_services::test_support;
use uuid::Uuid;

/// A sealed `SystemAdmin` proof for the mechanics tests below. They exercise service behavior
/// (coalesce, lockout guards, promotion, auto-join enrollment), not the authz gate — but the acts now
/// require the proof, so mint a real one (admin-authz enclosure, spec §3).
async fn admin_proof(pool: &sqlx::PgPool) -> temper_services::auth::SystemAdmin {
    test_support::system_admin_proof(pool).await
}

/// Seed the singleton settings row to a known baseline (the seed migration
/// inserts `id=1` already, but be explicit so the test is self-contained).
async fn reset_settings(pool: &sqlx::PgPool) {
    sqlx::query(
        "UPDATE kb_system_settings \
         SET gating_team_slug=NULL, instance_name=NULL, \
             terms_version=NULL, terms_resource_uri=NULL WHERE id=1",
    )
    .execute(pool)
    .await
    .expect("reset settings");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_settings_partial_coalesces(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    let admin = admin_proof(&pool).await;

    let req = UpdateSettingsRequest {
        instance_name: Some("Acme Temper".to_owned()),
        ..Default::default()
    };
    let updated = access_service::update_system_settings(&pool, &admin, &req)
        .await
        .expect("update");

    assert_eq!(updated.instance_name.as_deref(), Some("Acme Temper"));
    // (the COALESCE "untouched field preserved" check previously asserted on `access_mode`, which
    // Phase 2 removed from the settings wire type; instance_name above carries the same guarantee.)
}

// The `access_mode` control is retired (spec §14 / D18): `update_system_settings` no longer accepts
// it, so the old "rejects unknown access_mode" and "invite_only requires a gating team" tests are
// gone with the behaviors they pinned. The lockout they guarded can no longer happen — Task 7's
// repoint made `has_system_access` read standing, not gating-team membership. The gating-team-exists
// guard survives, decoupled from the mode; it is exercised by
// `update_settings_rejects_nonexistent_gating_team` below.

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_defaults_to_gating_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    let admin = admin_proof(&pool).await;
    // Configure a gating team that exists.
    let team_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(&pool)
        .await
        .expect("set gating");

    let profile = common::fixtures::create_test_profile(&pool, "promotee@test.example.com").await;

    let row = access_service::promote_admin(&pool, &admin, profile, None)
        .await
        .expect("promote");

    assert_eq!(row.team_id, team_id);
    assert_eq!(row.profile_id, profile);
    assert!(matches!(
        row.role,
        temper_core::types::team::TeamRole::Owner
    ));

    // is_system_admin now true for the promotee.
    let is_admin =
        access_service::is_system_admin(&pool, temper_core::types::ids::ProfileId::from(profile))
            .await
            .expect("check");
    assert!(is_admin);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_without_gating_or_team_is_bad_request(pool: sqlx::PgPool) {
    reset_settings(&pool).await; // gating_team_slug NULL, no --team
    let admin = admin_proof(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "x@test.example.com").await;
    let err = access_service::promote_admin(&pool, &admin, profile, None)
        .await
        .expect_err("no target team");
    assert!(matches!(
        err,
        temper_services::error::ApiError::BadRequest(_)
    ));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_settings_rejects_nonexistent_gating_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await; // no team named "does-not-exist"
    let admin = admin_proof(&pool).await;

    // The gating-team-exists guard survives the access_mode retirement, decoupled from any mode:
    // pointing the gating slug at a nonexistent team would silently break admin resolution
    // (`is_system_admin` reads governance keyed on that team's ownership).
    let req = UpdateSettingsRequest {
        gating_team_slug: Some("does-not-exist".to_owned()),
        ..Default::default()
    };
    let err = access_service::update_system_settings(&pool, &admin, &req)
        .await
        .expect_err("a nonexistent gating team should be rejected");
    assert!(matches!(
        err,
        temper_services::error::ApiError::BadRequest(_)
    ));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_rejects_nonexistent_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    let admin = admin_proof(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "p@test.example.com").await;
    // Pass a random team_id that does not exist in kb_teams.
    let bad_team_id = Uuid::new_v4();
    let err = access_service::promote_admin(&pool, &admin, profile, Some(bad_team_id))
        .await
        .expect_err("explicit nonexistent team should be rejected");
    assert!(matches!(
        err,
        temper_services::error::ApiError::BadRequest(_)
    ));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_rejects_nonexistent_profile(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    let admin = admin_proof(&pool).await;
    // Configure a real gating team so the None branch resolves.
    sqlx::query(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name",
    )
    .execute(&pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(&pool)
        .await
        .expect("set gating");

    // Pass a random profile_id that does not exist in kb_profiles.
    let bad_profile_id = Uuid::new_v4();
    let err = access_service::promote_admin(&pool, &admin, bad_profile_id, None)
        .await
        .expect_err("nonexistent profile should be rejected");
    assert!(matches!(
        err,
        temper_services::error::ApiError::BadRequest(_)
    ));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn approval_enrolls_into_other_auto_join_teams(pool: sqlx::PgPool) {
    // Gating team = temper-system (auto_join_role watcher, seeded by migration).
    let gating_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("gating team");
    // A SECOND auto-join team that is NOT the gating team — proves the hook does
    // more than the direct gating-team insert.
    let other_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name, auto_join_role) \
         VALUES ('everyone','Everyone','member') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("other auto-join team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(&pool)
        .await
        .expect("invite_only");

    let joiner = common::fixtures::create_test_profile(&pool, "joiner@test.example.com").await;

    // Joiner submits a request for the gating team.
    let request_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, status, source) \
         VALUES (gen_random_uuid(), $1, $2, 'pending', 'test') RETURNING id",
    )
    .bind(gating_id)
    .bind(joiner)
    .fetch_one(&pool)
    .await
    .expect("join request");

    // An operator approves via the service. The reviewer recorded on the decision is now the
    // authorizing admin (`admin.actor()`), so the proof IS the reviewer.
    let admin = admin_proof(&pool).await;
    access_service::review_request(
        &pool,
        &admin,
        access_service::ReviewRequestParams {
            request_id,
            decision: temper_core::types::access_gate::JoinRequestStatus::Approved,
            decision_note: None,
        },
    )
    .await
    .expect("approve");

    // The joiner is now enrolled in the OTHER auto-join team via the hook.
    let in_other: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kb_team_members WHERE team_id=$1 AND profile_id=$2)",
    )
    .bind(other_id)
    .bind(joiner)
    .fetch_one(&pool)
    .await
    .expect("check");
    assert!(
        in_other,
        "approval should enroll the profile into auto-join teams"
    );
}
