#![cfg(feature = "test-db")]
//! Integration tests for SAML membership reconcile. Each test runs on an isolated
//! `#[sqlx::test]` database with the workspace migrations applied.

use sqlx::PgPool;
use temper_core::types::TeamRole;
use temper_services::services::saml_provisioning_service::reconcile_idp_memberships;
use uuid::Uuid;

/// Minimal fixtures: a profile, two teams, one IdP, and mappings. Returns (profile, team_a, team_b).
async fn seed(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("user-{}", Uuid::now_v7()))
    .fetch_one(pool)
    .await
    .unwrap();

    let team_a: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("eng-{}", Uuid::now_v7()))
    .fetch_one(pool)
    .await
    .unwrap();

    let team_b: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("ops-{}", Uuid::now_v7()))
    .fetch_one(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO kb_saml_idp (idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id, sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr, groups_attr)
         VALUES ('acme', true, 'x', 'https://idp/sso', 'idp', 'sp', 'https://sp/acs', 'persistent', 'email', 'uid', 'groups')",
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) VALUES
         ('acme', 'engineering', $1, 'member'),
         ('acme', 'eng-leads',   $1, 'maintainer'),
         ('acme', 'operations',  $2, 'member')",
    )
    .bind(team_a)
    .bind(team_b)
    .execute(pool)
    .await
    .unwrap();

    (profile, team_a, team_b)
}

async fn membership(pool: &PgPool, team: Uuid, profile: Uuid) -> Option<(String, String)> {
    sqlx::query_as::<_, (TeamRole, String)>(
        "SELECT role, source::text FROM kb_team_members WHERE team_id=$1 AND profile_id=$2",
    )
    .bind(team)
    .bind(profile)
    .fetch_optional(pool)
    .await
    .unwrap()
    .map(|(r, s)| (format!("{r:?}"), s))
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn adds_idp_memberships_and_picks_max_role(pool: PgPool) {
    let (profile, team_a, team_b) = seed(&pool).await;

    let out = reconcile_idp_memberships(
        &pool,
        profile,
        "acme",
        &[
            "engineering".into(),
            "eng-leads".into(),
            "operations".into(),
        ],
    )
    .await
    .unwrap();

    assert_eq!(out.added, 2);
    // engineering(member) + eng-leads(maintainer) collapse to Maintainer on team_a.
    assert_eq!(
        membership(&pool, team_a, profile).await,
        Some(("Maintainer".into(), "idp".into()))
    );
    assert_eq!(
        membership(&pool, team_b, profile).await,
        Some(("Member".into(), "idp".into()))
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revokes_idp_memberships_no_longer_asserted(pool: PgPool) {
    let (profile, team_a, _team_b) = seed(&pool).await;
    reconcile_idp_memberships(&pool, profile, "acme", &["engineering".into()])
        .await
        .unwrap();
    assert!(membership(&pool, team_a, profile).await.is_some());

    // Second login: no groups asserted -> the idp row is revoked.
    let out = reconcile_idp_memberships(&pool, profile, "acme", &[])
        .await
        .unwrap();
    assert_eq!(out.revoked, 1);
    assert_eq!(membership(&pool, team_a, profile).await, None);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn native_membership_is_never_touched(pool: PgPool) {
    let (profile, team_a, _team_b) = seed(&pool).await;
    // A native membership on team_a (e.g. a join request approval).
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role, source) VALUES ($1,$2,'owner','native')",
    )
    .bind(team_a)
    .bind(profile)
    .execute(&pool)
    .await
    .unwrap();

    // IdP asserts engineering (maps to team_a member) -> must skip; native owner survives.
    let out = reconcile_idp_memberships(&pool, profile, "acme", &["engineering".into()])
        .await
        .unwrap();
    assert_eq!(out.skipped_native, 1);
    assert_eq!(out.added, 0);
    assert_eq!(
        membership(&pool, team_a, profile).await,
        Some(("Owner".into(), "native".into()))
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn asserted_groups_are_captured_for_discovery_even_when_unmapped(pool: PgPool) {
    let (profile, _team_a, _team_b) = seed(&pool).await;
    // 'engineering' is mapped; 'ghosts' is NOT mapped — both must still be captured.
    reconcile_idp_memberships(
        &pool,
        profile,
        "acme",
        &["engineering".into(), "ghosts".into()],
    )
    .await
    .unwrap();

    let seen: Vec<String> = sqlx::query_scalar(
        "SELECT group_value FROM kb_saml_seen_groups WHERE idp_key = 'acme' ORDER BY group_value",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(seen, vec!["engineering".to_string(), "ghosts".to_string()]);
}
