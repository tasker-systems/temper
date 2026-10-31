#![cfg(feature = "test-db")]

//! The root and the gating team's lifecycle — taking a child, being renamed, being soft-deleted — is
//! a system admin's alone (temper-artifacts `specs/2026-10-30-team-authority-fixes-design.md` D4).
//!
//! `demote_admin` revokes governance and deliberately leaves the `owner` row `promote_admin` wrote on
//! the gating team. The rank rule already makes that row self-only on the roster (R11); these gates
//! make it inert on the team's lifecycle too, so a demoted admin holds nothing through it.

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_core::types::team::{TeamCreateRequest, TeamUpdateRequest};
use temper_services::error::ApiError;
use temper_services::services::{access_service, team_service};
use temper_services::test_support;
use temper_workflow::operations::Surface;
use uuid::Uuid;

async fn person(pool: &PgPool, handle: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("seed profile");
    for surface in Surface::ALL {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, $2, '{}'::jsonb)",
        )
        .bind(id)
        .bind(format!("{handle}@{}", surface.marker()))
        .execute(pool)
        .await
        .expect("seed emitter");
    }
    id
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_demoted_admins_leftover_owner_row_governs_nothing(pool: PgPool) {
    let gate: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), 'gate', 'gate') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'gate'")
        .execute(&pool)
        .await
        .unwrap();

    let root_admin = person(&pool, "a2-root").await;
    test_support::approved_admin(&pool, root_admin).await;
    let admin_proof = test_support::system_admin_proof_for(&pool, root_admin).await;

    let ex = person(&pool, "a2-ex").await;
    test_support::approve(&pool, ex).await;
    access_service::promote_admin(&pool, &admin_proof, ex, None)
        .await
        .expect("promote onto the gating team");
    access_service::demote_admin(&pool, &admin_proof, ProfileId::from(ex))
        .await
        .expect("demote");
    assert!(
        !access_service::is_system_admin(&pool, ex.into())
            .await
            .unwrap(),
        "fixture: ex is no longer an admin"
    );
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(gate)
    .bind(ex)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        role.as_deref(),
        Some("owner"),
        "fixture: demotion leaves the gating owner row"
    );

    let ex_principal = test_support::human_principal_for(&pool, ex).await;

    let child = TeamCreateRequest {
        slug: "a2-shadow".into(),
        name: None,
        parent: Some("+gate".into()),
        auto_join_role: None,
    };
    let refusal = team_service::create_team(&pool, &ex_principal, &child)
        .await
        .expect_err("a non-admin creates no child of the gating team");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");

    let rename = TeamUpdateRequest {
        name: Some("renamed".into()),
        description: None,
    };
    let refusal = team_service::update_team(&pool, &ex_principal, gate, &rename)
        .await
        .expect_err("a non-admin does not rename the gating team");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");

    let refusal = team_service::delete_team(&pool, &ex_principal, gate)
        .await
        .expect_err("a non-admin does not delete the gating team");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");

    // The admin does all three.
    let admin_principal = test_support::human_principal_for(&pool, root_admin).await;
    let made = team_service::create_team(&pool, &admin_principal, &child)
        .await
        .expect("an admin creates a child of the gating team");
    let reaches_gate: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM team_ancestors($1) WHERE team_id = $2)")
            .bind(made.id)
            .bind(gate)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(reaches_gate);
    team_service::update_team(&pool, &admin_principal, gate, &rename)
        .await
        .expect("an admin renames the gating team");
    team_service::delete_team(&pool, &admin_principal, gate)
        .await
        .expect("an admin soft-deletes the gating team");
}

/// The root behaves the same whether or not it is the configured gating team.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_root_takes_children_only_from_an_admin(pool: PgPool) {
    let root: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let ex = person(&pool, "a2-root-owner").await;
    test_support::approve(&pool, ex).await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner') \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = 'owner'",
    )
    .bind(root)
    .bind(ex)
    .execute(&pool)
    .await
    .unwrap();
    let p = test_support::human_principal_for(&pool, ex).await;
    let refusal = team_service::create_team(
        &pool,
        &p,
        &TeamCreateRequest {
            slug: "a2-under-root".into(),
            name: None,
            parent: Some("temper-system".into()),
            auto_join_role: None,
        },
    )
    .await
    .expect_err("a non-admin owner of the root creates no child of it");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
}

/// A personal team takes no children, and an admin deletes a team they do not own.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn no_child_under_a_personal_team_and_admins_delete_any_team(pool: PgPool) {
    let me = person(&pool, "a2-me").await;
    test_support::approve(&pool, me).await;
    let p = test_support::human_principal_for(&pool, me).await;
    let personal: String = sqlx::query_scalar("SELECT slug FROM kb_teams WHERE personal_of = $1")
        .bind(me)
        .fetch_one(&pool)
        .await
        .unwrap();
    let refusal = team_service::create_team(
        &pool,
        &p,
        &TeamCreateRequest {
            slug: "a2-under-personal".into(),
            name: None,
            parent: Some(personal),
            auto_join_role: None,
        },
    )
    .await
    .expect_err("no child under a personal team");
    assert!(matches!(refusal, ApiError::BadRequest(_)), "{refusal:?}");

    let mine = team_service::create_team(
        &pool,
        &p,
        &TeamCreateRequest {
            slug: "a2-mine".into(),
            name: None,
            parent: None,
            auto_join_role: None,
        },
    )
    .await
    .expect("a root team");
    let admin = person(&pool, "a2-admin").await;
    test_support::approved_admin(&pool, admin).await;
    let a = test_support::human_principal_for(&pool, admin).await;
    team_service::delete_team(&pool, &a, mine.id)
        .await
        .expect("an admin soft-deletes a team they do not own");
}
