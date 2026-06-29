#![cfg(feature = "test-db")]

mod common;

use temper_api::services::access_service;
use temper_core::types::admin::UpdateSettingsRequest;
use uuid::Uuid;

/// Seed the singleton settings row to a known baseline (the seed migration
/// inserts `id=1` already, but be explicit so the test is self-contained).
async fn reset_settings(pool: &sqlx::PgPool) {
    sqlx::query(
        "UPDATE kb_system_settings \
         SET access_mode='open', gating_team_slug=NULL, instance_name=NULL, \
             terms_version=NULL, terms_resource_uri=NULL WHERE id=1",
    )
    .execute(pool)
    .await
    .expect("reset settings");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_settings_partial_coalesces(pool: sqlx::PgPool) {
    reset_settings(&pool).await;

    let req = UpdateSettingsRequest {
        instance_name: Some("Acme Temper".to_owned()),
        ..Default::default()
    };
    let updated = access_service::update_system_settings(&pool, &req)
        .await
        .expect("update");

    assert_eq!(updated.instance_name.as_deref(), Some("Acme Temper"));
    assert_eq!(updated.access_mode, "open"); // untouched field preserved
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_settings_rejects_unknown_access_mode(pool: sqlx::PgPool) {
    reset_settings(&pool).await;

    let req = UpdateSettingsRequest {
        access_mode: Some("banana".to_owned()),
        ..Default::default()
    };
    let err = access_service::update_system_settings(&pool, &req)
        .await
        .expect_err("should reject");
    assert!(matches!(err, temper_api::error::ApiError::BadRequest(_)));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_settings_invite_only_requires_gating_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await; // gating_team_slug is NULL

    let req = UpdateSettingsRequest {
        access_mode: Some("invite_only".to_owned()),
        ..Default::default()
    };
    let err = access_service::update_system_settings(&pool, &req)
        .await
        .expect_err("invite_only without a gating team should be rejected");
    assert!(matches!(err, temper_api::error::ApiError::BadRequest(_)));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_defaults_to_gating_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
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

    let row = access_service::promote_admin(&pool, profile, None)
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
    let profile = common::fixtures::create_test_profile(&pool, "x@test.example.com").await;
    let err = access_service::promote_admin(&pool, profile, None)
        .await
        .expect_err("no target team");
    assert!(matches!(err, temper_api::error::ApiError::BadRequest(_)));
}
