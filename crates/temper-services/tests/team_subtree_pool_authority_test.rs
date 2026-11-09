#![cfg(feature = "test-db")]

//! Bringing a pool under a team is an admin's act, over the whole subtree.
//!
//! An auto-join team's roster is every approved profile, and the gating team's roster is the access
//! pool. So putting either one *anywhere* below a team hands that team's material to the whole
//! instance. These tests pin the subtree form: the act is a system admin's when the
//! child's subtree holds one, live or dead. A soft-deleted pool below confers nothing today, but it
//! is latent reach (an admin's later restore hands the parent to the instance), so a non-admin may
//! not set it up. Restoring a team that keeps a link to a live root or gating parent is a system
//! admin's act too, as attaching under one is.
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

/// A soft-deleted pool grandchild confers nothing upward today, but restoring it would hand the
/// parent to the instance: it makes the attach an admin's act, and the reparent too. A dead team
/// with no pool below it does not.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_dead_pool_below_counts(pool: PgPool) {
    let f = fixture(&pool).await;
    let refusal = team_service::attach_child(&pool, &f.mgr, f.hub, f.top2)
        .await
        .expect_err("a non-admin brings a dead pool under hub");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!linked(&pool, f.hub, f.top2).await, "nothing is written");

    // top4 ─▸ dead mid ─▸ the gating team: the gating team below a dead team counts as well.
    let (top4, mid4, _gate4) = chain(&pool, &f.admin, None, "sp4", "g", None).await;
    own(&pool, top4, *f.mgr.profile_id()).await;
    team_service::delete_team(&pool, &f.admin, mid4)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'sp4-g'")
        .execute(&pool)
        .await
        .unwrap();
    let refusal = team_service::attach_child(&pool, &f.mgr, f.hub, top4)
        .await
        .expect_err("a non-admin brings a gating team below a dead team under hub");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'sp3-gate'")
        .execute(&pool)
        .await
        .unwrap();

    let old2 = create(&pool, &f.admin, "sp-old2", None, None).await;
    own(&pool, old2, *f.mgr.profile_id()).await;
    team_service::attach_child(&pool, &f.admin, old2, f.top2)
        .await
        .unwrap();
    let refusal = team_service::reparent(&pool, &f.mgr, f.top2, &reparent_req(old2, f.hub))
        .await
        .expect_err("a non-admin moves a dead pool under hub");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(linked(&pool, old2, f.top2).await, "nothing moved");

    let done = team_service::attach_child(&pool, &f.admin, f.hub, f.top2)
        .await
        .expect("the admin may");
    assert_eq!(done.outcome, AttachOutcome::Linked);

    // A dead team with nothing pool-like below it is no admin's act.
    let (top5, mid5, _) = chain(&pool, &f.admin, None, "sp5", "leaf", None).await;
    own(&pool, top5, *f.mgr.profile_id()).await;
    team_service::delete_team(&pool, &f.admin, mid5)
        .await
        .unwrap();
    let done = team_service::attach_child(&pool, &f.mgr, f.hub, top5)
        .await
        .expect("a dead subtree with no pool");
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

/// Restoring a team whose subtree holds an auto-join team, live or dead, re-links that pool's reach
/// (or its latent reach) to everything above, so it is an admin's act.
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

    let refusal = team_service::restore_team(&pool, &f.mgr, f.rtop2)
        .await
        .expect_err("a dead pool below counts");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!live(&pool, f.rtop2).await, "nothing restored");
    team_service::restore_team(&pool, &f.admin, f.rtop2)
        .await
        .expect("the admin may");
    assert!(live(&pool, f.rtop2).await);
}

/// A team made by an admin under `parent`, owned by the manager, then soft-deleted by the admin.
async fn dead_child_of(pool: &PgPool, f: &Fixture, slug: &str, parent: &str) -> Uuid {
    let t = create(pool, &f.admin, slug, Some(parent), None).await;
    own(pool, t, *f.mgr.profile_id()).await;
    team_service::delete_team(pool, &f.admin, t).await.unwrap();
    t
}

/// Restoring a team that keeps a link to the root or the gating team, live or dead, re-links it
/// under an instance team, which is attach's admin-only rule: the owner is refused, a system admin restores it and
/// the outcome names that parent. Under an ordinary parent the owner still restores, and so does
/// a personal team's owner, whose team's only parent is the root.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn restoring_under_the_root_or_the_gating_team_is_an_admins_act(pool: PgPool) {
    let f = fixture(&pool).await;
    for (slug, parent) in [("spi-root", "temper-system"), ("spi-gate", "sp3-gate")] {
        let t = dead_child_of(&pool, &f, slug, parent).await;
        let refusal = team_service::restore_team(&pool, &f.mgr, t)
            .await
            .expect_err("the owner restores under an instance parent");
        assert!(
            matches!(refusal, ApiError::Forbidden),
            "{slug}: {refusal:?}"
        );
        assert!(!live(&pool, t).await, "{slug}: nothing restored");
        let done = team_service::restore_team(&pool, &f.admin, t)
            .await
            .expect("the admin may");
        assert!(
            done.parents.iter().any(|p| p.slug == parent),
            "{slug}: {:?}",
            done.parents
        );
    }

    // A dead gating parent counts too: an admin's later restore of it would complete the link.
    let gate2 = create(&pool, &f.admin, "spi-gate2", None, None).await;
    let t = dead_child_of(&pool, &f, "spi-under-dead-gate", "spi-gate2").await;
    team_service::delete_team(&pool, &f.admin, gate2)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'spi-gate2'")
        .execute(&pool)
        .await
        .unwrap();
    let refusal = team_service::restore_team(&pool, &f.mgr, t)
        .await
        .expect_err("the owner restores under a dead gating parent");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'sp3-gate'")
        .execute(&pool)
        .await
        .unwrap();

    let t = dead_child_of(&pool, &f, "spi-plain", "sp-hub").await;
    team_service::restore_team(&pool, &f.mgr, t)
        .await
        .expect("an ordinary parent");
    assert!(live(&pool, t).await);

    let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(*f.mgr.profile_id())
        .fetch_one(&pool)
        .await
        .expect("the manager's personal team");
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(personal)
        .execute(&pool)
        .await
        .unwrap();
    team_service::restore_team(&pool, &f.mgr, personal)
        .await
        .expect("a personal team's owner restores it");
    assert!(live(&pool, personal).await);
}

/// The parent check is decided again under the locks: a gating slug repointed at the dead team's
/// parent while the owner's restore waits is seen, and the restore is refused.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_parent_made_the_gating_team_while_the_restore_waits_is_seen(pool: PgPool) {
    let f = fixture(&pool).await;
    let t = dead_child_of(&pool, &f, "spw-child", "sp-hub").await;

    let mut holder = pool.begin().await.unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'sp-hub' WHERE id = 1")
        .execute(&mut *holder)
        .await
        .expect("the admin's repoint, uncommitted");
    let pool2 = pool.clone();
    let mgr = f.mgr.clone();
    let contender = tokio::spawn(async move { team_service::restore_team(&pool2, &mgr, t).await });
    await_gating_waiter(&pool).await;
    holder.commit().await.unwrap();

    let refusal = contender
        .await
        .unwrap()
        .expect_err("the parent is the gating team by the time the re-check runs");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!live(&pool, t).await, "nothing restored");
}

/// The re-check under the locks sees a pool linked into the subtree concurrently. An admin's
/// transaction links pool P under grandchild G (and holds G's row) while a non-admin attaches G's
/// grandparent D under the hub. The non-admin's act locks D's subtree, so it waits for the
/// admin's commit, re-checks, finds P below D, and is refused. Without the subtree locks it would
/// read the subtree before P arrived and land P under the hub.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_pool_linked_under_the_subtree_concurrently_is_seen_by_the_recheck(pool: PgPool) {
    let f = fixture(&pool).await;
    let (d, _x, g) = chain(&pool, &f.admin, None, "spl", "g", None).await;
    let pool_team = create(&pool, &f.admin, "spl-pool", None, Some(TeamRole::Watcher)).await;
    own(&pool, d, *f.mgr.profile_id()).await;

    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(g)
        .execute(&mut *holder)
        .await
        .unwrap();
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(g)
        .bind(pool_team)
        .execute(&mut *holder)
        .await
        .expect("the admin's link, uncommitted");

    let pool2 = pool.clone();
    let mgr = f.mgr.clone();
    let hub = f.hub;
    let contender =
        tokio::spawn(async move { team_service::attach_child(&pool2, &mgr, hub, d).await });
    test_support::await_team_lock_waiter(&pool).await;
    holder.commit().await.unwrap();

    let refusal = contender
        .await
        .unwrap()
        .expect_err("the pool is in the subtree by the time the re-check runs");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!linked(&pool, f.hub, d).await, "nothing is written");
}

/// The same race through a DEAD team. D is soft-deleted, below R. An admin links a pool P under D
/// (holding D's row) while a non-admin attaches R under the hub. A pool below a dead team counts,
/// and the non-admin's lock set covers dead teams, so it waits for the link, re-checks, finds P
/// below R through D, and is refused.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_pool_linked_below_a_dead_team_concurrently_is_seen_by_the_recheck(pool: PgPool) {
    let f = fixture(&pool).await;
    let (r, _x, dead) = chain(&pool, &f.admin, None, "spd", "d", None).await;
    let pool_team = create(&pool, &f.admin, "spd-pool", None, Some(TeamRole::Watcher)).await;
    team_service::delete_team(&pool, &f.admin, dead)
        .await
        .expect("soft-delete D");
    own(&pool, r, *f.mgr.profile_id()).await;

    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(dead)
        .execute(&mut *holder)
        .await
        .unwrap();
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(dead)
        .bind(pool_team)
        .execute(&mut *holder)
        .await
        .expect("the admin's link under the dead team, uncommitted");

    let pool2 = pool.clone();
    let mgr = f.mgr.clone();
    let hub = f.hub;
    let contender =
        tokio::spawn(async move { team_service::attach_child(&pool2, &mgr, hub, r).await });
    test_support::await_team_lock_waiter(&pool).await;
    holder.commit().await.unwrap();

    let refusal = contender
        .await
        .unwrap()
        .expect_err("the pool is below the dead team by the time the re-check runs");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!linked(&pool, f.hub, r).await, "nothing is written");
}

/// A subtree that grows while the act waits for its locks, by a team that is no pool, is refused
/// as retryable (`409`), never acted on: the new team's own subtree was not locked.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_subtree_that_grows_while_the_act_waits_is_refused_as_retryable(pool: PgPool) {
    let f = fixture(&pool).await;
    let (d, _x, g) = chain(&pool, &f.admin, None, "spg", "g", None).await;
    let plain = create(&pool, &f.admin, "spg-plain", None, None).await;
    own(&pool, d, *f.mgr.profile_id()).await;

    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(g)
        .execute(&mut *holder)
        .await
        .unwrap();
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(g)
        .bind(plain)
        .execute(&mut *holder)
        .await
        .expect("a concurrent link, uncommitted");

    let pool2 = pool.clone();
    let mgr = f.mgr.clone();
    let hub = f.hub;
    let contender =
        tokio::spawn(async move { team_service::attach_child(&pool2, &mgr, hub, d).await });
    test_support::await_team_lock_waiter(&pool).await;
    holder.commit().await.unwrap();

    let refusal = contender
        .await
        .unwrap()
        .expect_err("the subtree changed under the act");
    assert!(matches!(refusal, ApiError::Conflict(_)), "{refusal:?}");
    assert!(!linked(&pool, f.hub, d).await, "nothing is written");
    // A retry, against the settled tree, succeeds: the new team is no pool.
    team_service::attach_child(&pool, &f.mgr, f.hub, d)
        .await
        .expect("the retry");
}

/// An admin's uncommitted link of `child` under `parent`, holding `parent`'s row as every link
/// does: the concurrent half of the race tests below.
async fn hold_link(
    pool: &PgPool,
    parent: Uuid,
    child: Uuid,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(parent)
        .execute(&mut *holder)
        .await
        .unwrap();
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(&mut *holder)
        .await
        .expect("the concurrent link, uncommitted");
    holder
}

/// A reparent `D: from → hub` by a non-admin, raced against an admin linking `leaf` under D's
/// grandchild. Returns the act's result once the admin has committed.
async fn race_reparent(
    pool: &PgPool,
    f: &Fixture,
    p: &str,
    leaf: Uuid,
) -> (Uuid, Result<(), ApiError>) {
    let from = create(pool, &f.admin, &format!("{p}-from"), None, None).await;
    let (d, _x, g) = chain(pool, &f.admin, Some(&format!("{p}-from")), p, "g", None).await;
    own(pool, from, *f.mgr.profile_id()).await;
    own(pool, d, *f.mgr.profile_id()).await;
    let holder = hold_link(pool, g, leaf).await;
    let pool2 = pool.clone();
    let mgr = f.mgr.clone();
    let req = reparent_req(from, f.hub);
    let contender = tokio::spawn(async move {
        team_service::reparent(&pool2, &mgr, d, &req)
            .await
            .map(|_| ())
    });
    test_support::await_team_lock_waiter(pool).await;
    holder.commit().await.unwrap();
    (d, contender.await.unwrap())
}

/// A restore of dead R by its owner, raced against an admin linking `leaf` under R's grandchild.
async fn race_restore(
    pool: &PgPool,
    f: &Fixture,
    p: &str,
    leaf: Uuid,
) -> (Uuid, Result<(), ApiError>) {
    let (r, _x, g) = chain(pool, &f.admin, None, p, "g", None).await;
    own(pool, r, *f.mgr.profile_id()).await;
    team_service::delete_team(pool, &f.admin, r)
        .await
        .expect("soft-delete R");
    let holder = hold_link(pool, g, leaf).await;
    let pool2 = pool.clone();
    let mgr = f.mgr.clone();
    let contender = tokio::spawn(async move {
        team_service::restore_team(&pool2, &mgr, r)
            .await
            .map(|_| ())
    });
    test_support::await_team_lock_waiter(pool).await;
    holder.commit().await.unwrap();
    (r, contender.await.unwrap())
}

/// The reparent re-check under the locks sees a pool linked into the subtree concurrently.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_pool_linked_under_a_reparented_subtree_concurrently_is_seen(pool: PgPool) {
    let f = fixture(&pool).await;
    let pool_team = create(&pool, &f.admin, "rrp-pool", None, Some(TeamRole::Watcher)).await;
    let (d, result) = race_reparent(&pool, &f, "rrp", pool_team).await;
    let refusal = result.expect_err("the pool is in the subtree by the time the re-check runs");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!linked(&pool, f.hub, d).await, "nothing moved");
}

/// A reparented subtree that grows while the act waits, by a team that is no pool, is a
/// retryable 409; the retry against the settled tree succeeds.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_reparented_subtree_that_grows_while_the_act_waits_is_retryable(pool: PgPool) {
    let f = fixture(&pool).await;
    let plain = create(&pool, &f.admin, "rrg-plain", None, None).await;
    let (d, result) = race_reparent(&pool, &f, "rrg", plain).await;
    let refusal = result.expect_err("the subtree changed under the act");
    assert!(matches!(refusal, ApiError::Conflict(_)), "{refusal:?}");
    assert!(!linked(&pool, f.hub, d).await, "nothing moved");
    let from: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'rrg-from'")
        .fetch_one(&pool)
        .await
        .unwrap();
    team_service::reparent(&pool, &f.mgr, d, &reparent_req(from, f.hub))
        .await
        .expect("the retry");
    assert!(linked(&pool, f.hub, d).await);
}

/// The restore re-check under the locks sees a pool linked below the dead team concurrently.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_pool_linked_under_a_restored_team_concurrently_is_seen(pool: PgPool) {
    let f = fixture(&pool).await;
    let pool_team = create(&pool, &f.admin, "rsp-pool", None, Some(TeamRole::Watcher)).await;
    let (r, result) = race_restore(&pool, &f, "rsp", pool_team).await;
    let refusal = result.expect_err("the pool is below R by the time the re-check runs");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!live(&pool, r).await, "nothing restored");
}

/// A restored team's subtree that grows while the act waits, by a team that is no pool, is a
/// retryable 409; the retry succeeds.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_restored_subtree_that_grows_while_the_act_waits_is_retryable(pool: PgPool) {
    let f = fixture(&pool).await;
    let plain = create(&pool, &f.admin, "rsg-plain", None, None).await;
    let (r, result) = race_restore(&pool, &f, "rsg", plain).await;
    let refusal = result.expect_err("the subtree changed under the act");
    assert!(matches!(refusal, ApiError::Conflict(_)), "{refusal:?}");
    assert!(!live(&pool, r).await, "nothing restored");
    team_service::restore_team(&pool, &f.mgr, r)
        .await
        .expect("the retry");
    assert!(live(&pool, r).await);
}

/// The gating slug is held for the re-check. An admin's settings update repoints
/// `gating_team_slug` at G, a team in D's subtree, and is held open (the exclusive gating lock, then
/// the settings row, as its statement trigger takes them) while a non-admin attaches D under the
/// hub.
/// The act takes its team rows, then waits on the shared gating hold; once the admin commits, the
/// re-check sees the gating team below D and refuses.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_gating_slug_repointed_into_the_subtree_concurrently_is_seen(pool: PgPool) {
    let f = fixture(&pool).await;
    let (d, _x, _g) = chain(&pool, &f.admin, None, "spq", "g", None).await;
    own(&pool, d, *f.mgr.profile_id()).await;

    let mut holder = pool.begin().await.unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'spq-g' WHERE id = 1")
        .execute(&mut *holder)
        .await
        .expect("the admin's repoint, uncommitted");

    let pool2 = pool.clone();
    let mgr = f.mgr.clone();
    let hub = f.hub;
    let contender =
        tokio::spawn(async move { team_service::attach_child(&pool2, &mgr, hub, d).await });
    await_gating_waiter(&pool).await;
    holder.commit().await.unwrap();

    let refusal = contender
        .await
        .unwrap()
        .expect_err("the gating team is in the subtree by the time the re-check runs");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!linked(&pool, f.hub, d).await, "nothing is written");
}

/// The same repoint in a rename's lock order, which is the order that must not deadlock: the
/// admin holds G's row first (as `rename_slug` does), the act waits on it, and only then does the
/// admin take the exclusive gating lock and repoint the slug. The act holds no gating lock while
/// it waits on a row, so the admin is never blocked by it; the act then re-checks and refuses.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_repoint_in_rename_order_does_not_deadlock_the_act(pool: PgPool) {
    let f = fixture(&pool).await;
    let (d, _x, g) = chain(&pool, &f.admin, None, "srn", "g", None).await;
    own(&pool, d, *f.mgr.profile_id()).await;

    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(g)
        .execute(&mut *holder)
        .await
        .unwrap();

    let pool2 = pool.clone();
    let mgr = f.mgr.clone();
    let hub = f.hub;
    let contender =
        tokio::spawn(async move { team_service::attach_child(&pool2, &mgr, hub, d).await });
    test_support::await_team_lock_waiter(&pool).await;
    sqlx::query("SELECT _gating_team_lock(true)")
        .execute(&mut *holder)
        .await
        .expect("the exclusive gating lock: the act must not be holding the shared side");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'srn-g' WHERE id = 1")
        .execute(&mut *holder)
        .await
        .expect("the repoint");
    holder.commit().await.unwrap();

    let refusal = contender
        .await
        .unwrap()
        .expect_err("the gating team is in the subtree by the time the re-check runs");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!linked(&pool, f.hub, d).await, "nothing is written");
}

/// Wait until some backend is blocked acquiring the shared gating hold.
async fn await_gating_waiter(pool: &PgPool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock' \
                AND query ILIKE '%_gating_team_lock(false)%')",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if waiting {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the contender never waited on the gating hold"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
