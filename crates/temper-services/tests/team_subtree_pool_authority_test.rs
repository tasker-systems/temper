#![cfg(feature = "test-db")]

//! R6 over the whole subtree (PW-1; temper-artifacts `plans/2026-11-06-public-within.md` Phase 0.2).
//!
//! An auto-join team's roster is every approved profile, and the gating team's roster is the access
//! pool. So putting either one *anywhere* below a team hands that team's material to the whole
//! instance. Attach, reparent and restore used to ask only whether the child (or the restored team)
//! was itself such a team. These tests pin the subtree form: the act is a system admin's when the
//! child's live subtree holds one, and a soft-deleted pool team below does not count, because reach
//! does not walk through a dead team either.
//!
//! The fixture (every team made by the admin, then `mgr` made owner where a non-admin needs it):
//!
//! ```text
//!   hub                      (the attach / reparent target; mgr owns it)
//!   old ─▸ top ─▸ mid ─▸ pool        pool: auto-join (watcher)            mgr owns old, top
//!   top2 ─▸ mid2 ─▸ deadpool         deadpool: auto-join, soft-deleted     mgr owns top2
//!   top3 ─▸ mid3 ─▸ gate             gate: the configured gating team      mgr owns top3
//!   rtop ─▸ rmid ─▸ rpool            rtop soft-deleted; rpool auto-join    mgr owns rtop
//!   rtop2 ─▸ rmid2 ─▸ rdeadpool      rtop2 and rdeadpool soft-deleted      mgr owns rtop2
//! ```

use sqlx::PgPool;
use temper_core::types::team::{AttachOutcome, ReparentRequest, TeamCreateRequest, TeamRole};
use temper_services::auth::HumanPrincipal;
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

async fn own(pool: &PgPool, team: Uuid, who: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner') \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team)
    .bind(who)
    .execute(pool)
    .await
    .expect("seed owner");
}

async fn create(
    pool: &PgPool,
    by: &HumanPrincipal,
    slug: &str,
    parent: Option<&str>,
    auto_join_role: Option<TeamRole>,
) -> Uuid {
    team_service::create_team(
        pool,
        by,
        &TeamCreateRequest {
            slug: slug.into(),
            name: None,
            parent: parent.map(|p| format!("+{p}")),
            parents: None,
            auto_join_role,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("create {slug}: {e:?}"))
    .id
}

/// A three-team chain `<p>-top ─▸ <p>-mid ─▸ <p>-<leaf>`, under `above` when given.
async fn chain(
    pool: &PgPool,
    admin: &HumanPrincipal,
    above: Option<&str>,
    p: &str,
    leaf: &str,
    leaf_auto_join: Option<TeamRole>,
) -> (Uuid, Uuid, Uuid) {
    let top_slug = format!("{p}-top");
    let mid_slug = format!("{p}-mid");
    let top = create(pool, admin, &top_slug, above, None).await;
    let mid = create(pool, admin, &mid_slug, Some(&top_slug), None).await;
    let leaf = create(
        pool,
        admin,
        &format!("{p}-{leaf}"),
        Some(&mid_slug),
        leaf_auto_join,
    )
    .await;
    (top, mid, leaf)
}

struct Fixture {
    admin: HumanPrincipal,
    mgr: HumanPrincipal,
    hub: Uuid,
    old: Uuid,
    top: Uuid,
    top2: Uuid,
    top3: Uuid,
    rtop: Uuid,
    rtop2: Uuid,
}

async fn fixture(pool: &PgPool) -> Fixture {
    let admin_id = person(pool, "sp-admin").await;
    test_support::grant_governance(pool, admin_id).await;
    let admin = test_support::human_principal_for(pool, admin_id).await;
    let mgr_id = person(pool, "sp-mgr").await;

    let hub = create(pool, &admin, "sp-hub", None, None).await;
    let old = create(pool, &admin, "sp-old", None, None).await;
    let (top, _, _) = chain(
        pool,
        &admin,
        Some("sp-old"),
        "sp",
        "pool",
        Some(TeamRole::Watcher),
    )
    .await;
    let (top2, _, deadpool) = chain(
        pool,
        &admin,
        None,
        "sp2",
        "deadpool",
        Some(TeamRole::Watcher),
    )
    .await;
    let (top3, _, _) = chain(pool, &admin, None, "sp3", "gate", None).await;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'sp3-gate'")
        .execute(pool)
        .await
        .expect("configure the gating team");
    let (rtop, _, _) = chain(pool, &admin, None, "spr", "pool", Some(TeamRole::Watcher)).await;
    let (rtop2, _, rdeadpool) = chain(
        pool,
        &admin,
        None,
        "spr2",
        "deadpool",
        Some(TeamRole::Watcher),
    )
    .await;

    for team in [hub, old, top, top2, top3, rtop, rtop2] {
        own(pool, team, mgr_id).await;
    }
    for team in [deadpool, rdeadpool, rtop, rtop2] {
        team_service::delete_team(pool, &admin, team)
            .await
            .unwrap_or_else(|e| panic!("soft-delete {team}: {e:?}"));
    }
    let mgr = test_support::human_principal_for(pool, mgr_id).await;
    Fixture {
        admin,
        mgr,
        hub,
        old,
        top,
        top2,
        top3,
        rtop,
        rtop2,
    }
}

async fn linked(pool: &PgPool, parent: Uuid, child: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_teams_parents WHERE parent_id = $1 AND child_id = $2)",
    )
    .bind(parent)
    .bind(child)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn live(pool: &PgPool, team: Uuid) -> bool {
    sqlx::query_scalar("SELECT is_active FROM kb_teams WHERE id = $1")
        .bind(team)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn reparent_req(from: Uuid, to: Uuid) -> ReparentRequest {
    ReparentRequest {
        from: from.to_string(),
        to: to.to_string(),
    }
}

/// The primitive itself: live teams only, the seed included only while live, and nothing below a
/// dead team, mirroring `team_ancestors`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn team_live_descendants_walks_live_teams_only(pool: PgPool) {
    let f = fixture(&pool).await;
    let slugs = |team: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT t.slug FROM team_live_descendants($1) d JOIN kb_teams t ON t.id = d.team_id \
                 ORDER BY t.slug",
            )
            .bind(team)
            .fetch_all(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(
        slugs(f.old).await,
        vec!["sp-mid", "sp-old", "sp-pool", "sp-top"]
    );
    assert_eq!(
        slugs(f.top2).await,
        vec!["sp2-mid", "sp2-top"],
        "a soft-deleted descendant is not returned"
    );
    assert!(
        slugs(f.rtop).await.is_empty(),
        "a dead seed is neither returned nor walked through"
    );
}

/// Attaching a team whose GRANDCHILD is auto-join is a system admin's act: refused to a manager of
/// both sides, admitted to an admin. The same for a gating grandchild.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn attaching_a_subtree_that_holds_a_pool_is_an_admins_act(pool: PgPool) {
    let f = fixture(&pool).await;
    for child in [f.top, f.top3] {
        let refusal = team_service::attach_child(&pool, &f.mgr, f.hub, child)
            .await
            .expect_err("a non-admin brings a pool under hub");
        assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
        assert!(!linked(&pool, f.hub, child).await, "nothing is written");
    }
    for child in [f.top, f.top3] {
        let done = team_service::attach_child(&pool, &f.admin, f.hub, child)
            .await
            .expect("the admin may");
        assert_eq!(done.outcome, AttachOutcome::Linked);
    }
}

/// A soft-deleted pool grandchild confers nothing upward, so it does not make the attach an
/// admin's act.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_dead_pool_below_does_not_count(pool: PgPool) {
    let f = fixture(&pool).await;
    let done = team_service::attach_child(&pool, &f.mgr, f.hub, f.top2)
        .await
        .expect("the dead pool is not in the live subtree");
    assert_eq!(done.outcome, AttachOutcome::Linked);
}

/// Reparent reuses attach's checks, so moving a pool-holding subtree is an admin's act too.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reparenting_a_subtree_that_holds_a_pool_is_an_admins_act(pool: PgPool) {
    let f = fixture(&pool).await;
    let req = reparent_req(f.old, f.hub);
    let refusal = team_service::reparent(&pool, &f.mgr, f.top, &req)
        .await
        .expect_err("a non-admin moves a pool under hub");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(linked(&pool, f.old, f.top).await, "nothing moved");
    assert!(!linked(&pool, f.hub, f.top).await);

    team_service::reparent(&pool, &f.admin, f.top, &req)
        .await
        .expect("the admin may");
    assert!(linked(&pool, f.hub, f.top).await);
    assert!(!linked(&pool, f.old, f.top).await);
}

/// Restoring a team whose live subtree holds an auto-join team re-links that pool's reach to
/// everything above, so it is an admin's act; with only a dead pool below, the owner may restore.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn restoring_a_subtree_that_holds_a_pool_is_an_admins_act(pool: PgPool) {
    let f = fixture(&pool).await;
    let refusal = team_service::restore_team(&pool, &f.mgr, f.rtop)
        .await
        .expect_err("the owner restores a team with a live pool below");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!live(&pool, f.rtop).await, "nothing restored");

    team_service::restore_team(&pool, &f.admin, f.rtop)
        .await
        .expect("the admin may");
    assert!(live(&pool, f.rtop).await);

    team_service::restore_team(&pool, &f.mgr, f.rtop2)
        .await
        .expect("a dead pool below does not count");
    assert!(live(&pool, f.rtop2).await);
}
