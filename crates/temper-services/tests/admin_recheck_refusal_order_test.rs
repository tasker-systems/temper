#![cfg(feature = "test-db")]
//! A demoted admin holding a `&SystemAdmin` proof minted before the demotion is refused before the
//! request is judged, with the `403` a non-admin gets at the mint. Before this change
//! `promote_admin` and `update_system_settings` validated the request on the pool first, so the
//! stale proof learned whether a team, a profile or a gating-team slug existed (a `400` naming it)
//! before the `403`: a small existence signal inside the race window. Each test fails on the code
//! before this change, with that `400`.
//!
//! The admin re-check itself is unchanged and is raced in `admin_standing_verbs_recheck_test`;
//! these pin only where it sits. `promote_admin` keeps its lock order: the subject's standing row,
//! then governance, so a self-promotion and the promoter's own revoke still never wait in a cycle
//! (`a_demoted_admin_cannot_promote_themselves_back` there).

use sqlx::PgPool;
use temper_core::types::admin::UpdateSettingsRequest;
use temper_services::auth::SystemAdmin;
use temper_services::error::ApiError;
use temper_services::services::access_service;
use temper_services::test_support;
use uuid::Uuid;

async fn stale_proof(pool: &PgPool) -> (Uuid, SystemAdmin) {
    let admin: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), 'ro-admin', 'ro-admin') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, 'ro-admin@web', '{}')",
    )
    .bind(admin)
    .execute(pool)
    .await
    .unwrap();
    test_support::approved_admin(pool, admin).await;
    let proof = test_support::system_admin_proof_for(pool, admin).await;
    let _: bool = sqlx::query_scalar("SELECT principal_governance_set($1, false, NULL, 'test')")
        .bind(admin)
        .fetch_one(pool)
        .await
        .unwrap();
    (admin, proof)
}

fn forbidden(err: &ApiError) -> bool {
    matches!(err, ApiError::Forbidden)
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn promote_admin_refuses_a_stale_proof_before_it_judges_the_request(pool: PgPool) {
    let (admin, stale) = stale_proof(&pool).await;
    let real_team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('ro-team', 'ro-team') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // A team that does not exist: an admin is told `400 team '…' does not exist`.
    let err = access_service::promote_admin(&pool, &stale, admin, Some(Uuid::now_v7()))
        .await
        .unwrap_err();
    assert!(forbidden(&err), "a missing team: {err:?}");
    // A profile that does not exist: an admin is told `400 profile '…' does not exist`.
    let err = access_service::promote_admin(&pool, &stale, Uuid::now_v7(), Some(real_team))
        .await
        .unwrap_err();
    assert!(forbidden(&err), "a missing profile: {err:?}");
    // No team named, on an instance with no gating team configured: an admin is told `400`.
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = NULL")
        .execute(&pool)
        .await
        .unwrap();
    let err = access_service::promote_admin(&pool, &stale, admin, None)
        .await
        .unwrap_err();
    assert!(forbidden(&err), "no gating team: {err:?}");
    let governs: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_principal_governance WHERE profile_id = $1)",
    )
    .bind(admin)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!governs, "a refused promotion wrote a governance row");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn update_system_settings_refuses_a_stale_proof_before_it_judges_the_slug(pool: PgPool) {
    let (_, stale) = stale_proof(&pool).await;
    let before = access_service::get_system_settings(&pool).await.unwrap();
    // A slug that names no team: an admin is told `400 gating_team_slug '…' does not exist`.
    let err = access_service::update_system_settings(
        &pool,
        &stale,
        &UpdateSettingsRequest {
            gating_team_slug: Some("ro-no-such-team".into()),
            instance_name: Some("renamed".into()),
            terms_version: None,
            terms_resource_uri: None,
        },
    )
    .await
    .unwrap_err();
    assert!(forbidden(&err), "a missing slug: {err:?}");
    let after = access_service::get_system_settings(&pool).await.unwrap();
    assert_eq!(
        after.instance_name, before.instance_name,
        "a refused update wrote"
    );
}

/// The control: an admin still gets the validation answers, in the order they had.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_admin_still_gets_the_validation_answers(pool: PgPool) {
    let proof = test_support::system_admin_proof(&pool).await;
    let err = access_service::promote_admin(&pool, &proof, Uuid::now_v7(), Some(Uuid::now_v7()))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ApiError::BadRequest(why) if why.starts_with("team '")),
        "{err:?}"
    );
    let err = access_service::update_system_settings(
        &pool,
        &proof,
        &UpdateSettingsRequest {
            gating_team_slug: Some("ro-no-such-team".into()),
            instance_name: None,
            terms_version: None,
            terms_resource_uri: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::BadRequest(_)), "{err:?}");
}
