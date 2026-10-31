#![cfg(feature = "test-db")]

//! A parent team's managers see the teams beneath it and can cut them loose (temper-artifacts
//! `specs/2026-10-30-team-authority-fixes-design.md` D3).
//!
//! Rank stays per team (team-role rank spec R7): a parent role confers no say in a child's roster.
//! But a child's members reach everything attached to the parent, so the parent's managers get the
//! two things they would have over a member added to the parent directly: they can see who that is
//! (the child's roster), and they can end it (detach). Both link and unlink are recorded.
//!
//! ```text
//!   eng ─▸ tools          (tools created under eng by mallory, a maintainer of eng)
//!   alice: owner of eng
//!   mallory: maintainer of eng, owner of tools (its creator)
//!   eve: member of tools only
//!   bob: member of eng
//! ```

use sqlx::PgPool;
use temper_core::types::team::TeamCreateRequest;
use temper_services::error::ApiError;
use temper_services::services::team_service;
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
    test_support::approve(pool, id).await;
    id
}

async fn join(pool: &PgPool, team_id: Uuid, who: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role) \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team_id)
    .bind(who)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
}

async fn team_context(pool: &PgPool, owner_team: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(owner_team)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn reads_context(pool: &PgPool, who: Uuid, context: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM contexts_readable_by($1) c WHERE c.context_id = $2)",
    )
    .bind(who)
    .bind(context)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn link_events(pool: &PgPool, parent: Uuid, act: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_parent_link_changed' \
            AND e.payload @> jsonb_build_object('parent_team_id', $1::uuid, 'act', $2::text) \
            AND e.\"references\" @> jsonb_build_array(jsonb_build_object( \
                  'rel', 'subject', 'target', jsonb_build_object('kind', 'kb_teams', 'id', $1::uuid)))",
    )
    .bind(parent)
    .bind(act)
    .fetch_one(pool)
    .await
    .unwrap()
}

struct Fixture {
    eng: Uuid,
    tools: Uuid,
    alice: Uuid,
    mallory: Uuid,
    eve: Uuid,
    bob: Uuid,
}

async fn fixture(pool: &PgPool) -> Fixture {
    let alice = person(pool, "co-alice").await;
    let mallory = person(pool, "co-mallory").await;
    let eve = person(pool, "co-eve").await;
    let bob = person(pool, "co-bob").await;
    let a = test_support::human_principal_for(pool, alice).await;
    let eng = team_service::create_team(
        pool,
        &a,
        &TeamCreateRequest {
            slug: "co-eng".into(),
            name: None,
            parent: None,
            auto_join_role: None,
        },
    )
    .await
    .expect("alice creates eng")
    .id;
    join(pool, eng, mallory, "maintainer").await;
    join(pool, eng, bob, "member").await;
    let m = test_support::human_principal_for(pool, mallory).await;
    let tools = team_service::create_team(
        pool,
        &m,
        &TeamCreateRequest {
            slug: "co-tools".into(),
            name: None,
            parent: Some("+co-eng".into()),
            auto_join_role: None,
        },
    )
    .await
    .expect("a maintainer of eng creates a child")
    .id;
    join(pool, tools, eve, "member").await;
    Fixture {
        eng,
        tools,
        alice,
        mallory,
        eve,
        bob,
    }
}

/// Creating a child is recorded, and the parent's managers see the child and its roster — and
/// nothing of the child's own material.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_parents_managers_see_the_child_and_its_roster(pool: PgPool) {
    let f = fixture(&pool).await;
    assert_eq!(
        link_events(&pool, f.eng, "linked").await,
        1,
        "the link is recorded"
    );

    let alice = test_support::caller_for(&pool, f.alice).await;
    let eng = team_service::team_detail(&pool, &alice, f.eng)
        .await
        .unwrap();
    assert_eq!(eng.children.len(), 1);
    assert_eq!(eng.children[0].id, f.tools);

    // alice is not on tools, yet reads its roster through the ancestor arm.
    let tools = team_service::team_detail(&pool, &alice, f.tools)
        .await
        .expect("an ancestor's manager reads the child's roster");
    assert!(tools.members.iter().any(|m| m.profile_id == f.eve));

    // ...but not the child's own context: reach never flows up.
    let tools_ctx = team_context(&pool, f.tools, "co-tools-ctx").await;
    assert!(!reads_context(&pool, f.alice, tools_ctx).await);

    // A plain member of eng sees no children and no child roster.
    let bob = test_support::caller_for(&pool, f.bob).await;
    let eng_for_bob = team_service::team_detail(&pool, &bob, f.eng).await.unwrap();
    assert!(eng_for_bob.children.is_empty());
    let refusal = team_service::team_detail(&pool, &bob, f.tools)
        .await
        .expect_err("a member of the parent does not read the child");
    assert!(matches!(refusal, ApiError::NotFound(_)), "{refusal:?}");
}

/// The parent's owner detaches the child: its members stop reaching the parent, the child
/// survives, and the unlink is recorded.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_parents_manager_detaches_the_child_and_its_reach_ends(pool: PgPool) {
    let f = fixture(&pool).await;
    let eng_ctx = team_context(&pool, f.eng, "co-eng-ctx").await;
    assert!(
        reads_context(&pool, f.eve, eng_ctx).await,
        "fixture: eve reaches eng via tools"
    );

    let a = test_support::human_principal_for(&pool, f.alice).await;
    team_service::detach_child(&pool, &a, f.eng, f.tools)
        .await
        .expect("eng's owner detaches tools");

    assert!(
        !reads_context(&pool, f.eve, eng_ctx).await,
        "the reach ends"
    );
    let still: bool = sqlx::query_scalar("SELECT is_active FROM kb_teams WHERE id = $1")
        .bind(f.tools)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(still, "the child survives as a parentless team");
    assert_eq!(
        link_events(&pool, f.eng, "unlinked").await,
        1,
        "the unlink is recorded"
    );

    // A second detach finds nothing to cut.
    let refusal = team_service::detach_child(&pool, &a, f.eng, f.tools)
        .await
        .expect_err("already detached");
    assert!(matches!(refusal, ApiError::NotFound(_)), "{refusal:?}");
}

/// Only a manager of the PARENT detaches: not a member of it, and not the child's own owner.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn only_the_parents_managers_detach(pool: PgPool) {
    let f = fixture(&pool).await;
    for who in [f.bob, f.eve] {
        let p = test_support::human_principal_for(&pool, who).await;
        let refusal = team_service::detach_child(&pool, &p, f.eng, f.tools)
            .await
            .expect_err("a non-manager of the parent cannot detach");
        assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    }
    // mallory is a maintainer of eng, so she may (rank is per team; she manages the parent).
    let m = test_support::human_principal_for(&pool, f.mallory).await;
    team_service::detach_child(&pool, &m, f.eng, f.tools)
        .await
        .expect("a maintainer of the parent detaches");
}

/// The root never serves as the ancestor for a non-admin: a managing row on `temper-system` (a
/// demoted admin's) does not open every personal team's roster, nor list them as children.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_root_is_never_the_ancestor_for_a_non_admin(pool: PgPool) {
    let root: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let ex = person(&pool, "co-ex-admin").await;
    join(&pool, root, ex, "owner").await;
    let someone = person(&pool, "co-someone").await;
    let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(someone)
        .fetch_one(&pool)
        .await
        .unwrap();

    let c = test_support::caller_for(&pool, ex).await;
    let refusal = team_service::team_detail(&pool, &c, personal)
        .await
        .expect_err("no personal roster through the root");
    assert!(matches!(refusal, ApiError::NotFound(_)), "{refusal:?}");
    let root_detail = team_service::team_detail(&pool, &c, root).await.unwrap();
    assert!(
        root_detail.children.is_empty(),
        "the root lists no children to a non-admin"
    );

    let p = test_support::human_principal_for(&pool, ex).await;
    let refusal = team_service::detach_child(&pool, &p, root, personal)
        .await
        .expect_err("no detaching a personal team from the root");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
}
