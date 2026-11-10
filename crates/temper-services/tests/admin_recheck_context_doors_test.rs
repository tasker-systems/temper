#![cfg(feature = "test-db")]
//! The context doors that place a context with a team (share, unshare, reassign), restore (which
//! brings back every write grant and authoring arm on the context), and the cognitive-map binding
//! doors (bind, unbind) decided their system-admin arm on a pool read (`TwoSidedAuthority`,
//! `ContextAdminAuthority`) and never asked again on their write. Each now asks again on the write's
//! own transaction, under the governance lock (`auth::is_system_admin_locked`), before it writes.
//! A demoted admin is refused with the gate's answer to them as they now stand: the exact refusal a
//! non-admin with no role on either team gets.
//!
//! **In flight:** the admin's demotion is left open, the door is started, and the test waits until
//! it is blocked at `… kb_principal_governance … FOR KEY SHARE`; the pool reads ahead of it are
//! plain and never wait on an uncommitted delete. The demotion commits and the door is refused,
//! having written nothing. Every test fails on the code before this change: the door never blocked.
//! The committed-first half is a regression pin: these doors take a principal, not a proof, so
//! their pool read already sees a committed demotion.
//!
//! Lock order: governance first, then the share, binding or context row the act writes; a demotion
//! locks none of those.

use std::future::Future;

use sqlx::PgPool;
use temper_core::types::cognitive_maps::BindTeamRequest;
use temper_core::types::context::ShareContextRequest;
use temper_services::error::ApiError;
use temper_services::services::{cogmap_service, context_service};
use temper_services::test_support;
use temper_workflow::operations::Surface;
use uuid::Uuid;

async fn profile(pool: &PgPool, handle: &str) -> Uuid {
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

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("team")
}

async fn team_context(pool: &PgPool, team: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(team)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("context")
}

async fn cogmap(pool: &PgPool) -> Uuid {
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (id, name, telos_resource_id) \
         VALUES (uuid_generate_v7(), 'ra-map', $1) RETURNING id",
    )
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The context's identity and placement, for the "wrote nothing" assertions.
async fn context_row(pool: &PgPool, ctx: Uuid) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('owner', owner_id, 'slug', slug, 'name', name, \
                'active', is_active, 'shares', \
                (SELECT count(*) FROM kb_team_contexts WHERE context_id = c.id)) \
           FROM kb_contexts c WHERE id = $1",
    )
    .bind(ctx)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn bindings(pool: &PgPool, map: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_team_cogmaps WHERE cogmap_id = $1")
        .bind(map)
        .fetch_one(pool)
        .await
        .unwrap()
}

const GOVERNANCE_RECHECK: &str = "%FROM kb_principal_governance%FOR KEY SHARE%";

async fn demotion_in_flight(
    pool: &PgPool,
    who: Uuid,
) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut holder = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    let _: bool = sqlx::query_scalar("SELECT principal_governance_set($1, false, NULL, 'test')")
        .bind(who)
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    (holder, pid)
}

async fn await_blocked_at_the_recheck(pool: &PgPool, holder: i32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)) AND query ILIKE $2)",
        )
        .bind(holder)
        .bind(GOVERNANCE_RECHECK)
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the door never blocked at the governance re-check"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// Both races on `door`, run as `who`. `admin` is an admin again afterwards. Each refusal must be
/// the one `stranger`, a non-admin with no role on either team, gets at the same door.
async fn both_races<T, F, Fut>(pool: &PgPool, admin: Uuid, stranger: Uuid, door: F)
where
    T: std::fmt::Debug + Send + 'static,
    F: Fn(PgPool, Uuid) -> Fut,
    Fut: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    let refusal = format!("{:?}", door(pool.clone(), stranger).await.unwrap_err());

    let (holder, pid) = demotion_in_flight(pool, admin).await;
    let contender = tokio::spawn(door(pool.clone(), admin));
    await_blocked_at_the_recheck(pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert_eq!(format!("{err:?}"), refusal, "in flight");
    test_support::grant_governance(pool, admin).await;

    let (holder, _) = demotion_in_flight(pool, admin).await;
    holder.commit().await.unwrap();
    let err = door(pool.clone(), admin).await.unwrap_err();
    assert_eq!(format!("{err:?}"), refusal, "committed first");
    test_support::grant_governance(pool, admin).await;
}

struct Fixture {
    admin: Uuid,
    stranger: Uuid,
    ctx: Uuid,
    to: Uuid,
}

async fn fixture(pool: &PgPool) -> Fixture {
    let admin = profile(pool, "ra-admin").await;
    test_support::approved_admin(pool, admin).await;
    let stranger = profile(pool, "ra-stranger").await;
    let owner = team(pool, "ra-owner").await;
    let to = team(pool, "ra-to").await;
    let ctx = team_context(pool, owner, "ra-ctx").await;
    Fixture {
        admin,
        stranger,
        ctx,
        to,
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn share_and_unshare(pool: PgPool) {
    let Fixture {
        admin,
        stranger,
        ctx,
        to,
    } = fixture(&pool).await;
    let before = context_row(&pool, ctx).await;
    both_races(
        &pool,
        admin,
        stranger,
        move |pool: PgPool, who| async move {
            let by = test_support::human_principal_for(&pool, who).await;
            context_service::share(&pool, &by, ctx, &ShareContextRequest { team_id: to }).await
        },
    )
    .await;
    assert_eq!(
        context_row(&pool, ctx).await,
        before,
        "a refused share wrote"
    );

    let by = test_support::human_principal_for(&pool, admin).await;
    context_service::share(&pool, &by, ctx, &ShareContextRequest { team_id: to })
        .await
        .expect("the admin shares it");
    let shared = context_row(&pool, ctx).await;
    both_races(
        &pool,
        admin,
        stranger,
        move |pool: PgPool, who| async move {
            let by = test_support::human_principal_for(&pool, who).await;
            context_service::unshare(&pool, &by, ctx, to).await
        },
    )
    .await;
    assert_eq!(
        context_row(&pool, ctx).await,
        shared,
        "a refused unshare wrote"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reassign(pool: PgPool) {
    let Fixture {
        admin,
        stranger,
        ctx,
        to,
    } = fixture(&pool).await;
    let before = context_row(&pool, ctx).await;
    both_races(
        &pool,
        admin,
        stranger,
        move |pool: PgPool, who| async move {
            let by = test_support::human_principal_for(&pool, who).await;
            context_service::reassign(&pool, &by, ctx, to).await
        },
    )
    .await;
    assert_eq!(
        context_row(&pool, ctx).await,
        before,
        "a refused reassign moved the context"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn bind_and_unbind(pool: PgPool) {
    let Fixture {
        admin,
        stranger,
        to,
        ..
    } = fixture(&pool).await;
    let map = cogmap(&pool).await;
    both_races(
        &pool,
        admin,
        stranger,
        move |pool: PgPool, who| async move {
            let by = test_support::human_principal_for(&pool, who).await;
            cogmap_service::bind_team(&pool, &by, map, &BindTeamRequest { team_id: to }).await
        },
    )
    .await;
    assert_eq!(bindings(&pool, map).await, 0, "a refused bind wrote");

    let by = test_support::human_principal_for(&pool, admin).await;
    cogmap_service::bind_team(&pool, &by, map, &BindTeamRequest { team_id: to })
        .await
        .expect("the admin binds it");
    both_races(
        &pool,
        admin,
        stranger,
        move |pool: PgPool, who| async move {
            let by = test_support::human_principal_for(&pool, who).await;
            cogmap_service::unbind_team(&pool, &by, map, to).await
        },
    )
    .await;
    assert_eq!(
        bindings(&pool, map).await,
        1,
        "a refused unbind deleted the binding"
    );
}

/// Restore brings back every write grant and authoring arm on a retired context
/// (`context_authorable_by_profile` reads `is_active`; every explicit grant `context_is_live`). A
/// demoted admin's restore is refused with `ContextAdminAuthority`'s answer to them as they now
/// stand: the not-found sentence, since they cannot read the team's context, which is what the
/// stranger gets. A door that skipped asking the gate again would answer `403` and fail the
/// equality.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn restore(pool: PgPool) {
    let Fixture {
        admin,
        stranger,
        ctx,
        ..
    } = fixture(&pool).await;
    let by = test_support::caller_for(&pool, admin).await;
    context_service::retire(&pool, &by, ctx)
        .await
        .expect("the admin retires it");
    let retired = context_row(&pool, ctx).await;
    both_races(
        &pool,
        admin,
        stranger,
        move |pool: PgPool, who| async move {
            let by = test_support::caller_for(&pool, who).await;
            context_service::restore(&pool, &by, ctx).await
        },
    )
    .await;
    assert_eq!(
        context_row(&pool, ctx).await,
        retired,
        "a refused restore wrote"
    );
}

/// Unbinding a map from the gating team takes it out of the admin-only write regime
/// (`access_service::cogmap_write_requires_admin`): an escalation. A demoted admin's unbind is
/// refused, and the binding stays.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn unbind_from_the_gating_team(pool: PgPool) {
    let Fixture {
        admin, stranger, ..
    } = fixture(&pool).await;
    let gating = team(&pool, "ra-gating").await;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'ra-gating' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    let map = cogmap(&pool).await;
    let by = test_support::human_principal_for(&pool, admin).await;
    cogmap_service::bind_team(&pool, &by, map, &BindTeamRequest { team_id: gating })
        .await
        .expect("the admin binds it to the gating team");
    both_races(
        &pool,
        admin,
        stranger,
        move |pool: PgPool, who| async move {
            let by = test_support::human_principal_for(&pool, who).await;
            cogmap_service::unbind_team(&pool, &by, map, gating).await
        },
    )
    .await;
    assert_eq!(
        bindings(&pool, map).await,
        1,
        "a refused unbind took the map out of the admin-only regime"
    );
}
