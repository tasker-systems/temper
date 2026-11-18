#![cfg(feature = "test-db")]
//! A reassignment and a move of one resource serialize on R's writer lock
//! (`writes::lock_resource_for_write`, `FOR NO KEY UPDATE`): the lock `resource_rehome` takes, now
//! also taken by `writes::reassign_resource_in_tx` and, ahead of re-deciding authority, by both
//! reassign doors in `reassign_service`.
//!
//! Before it, the reassignment held only the liveness floor's `FOR KEY SHARE`, which a move's
//! `FOR NO KEY UPDATE` does not conflict with. `resource_reassign` read R's home unlocked, appended
//! `resource_reassigned` on it, and its projector then waited on the move's home-row update. A move
//! that committed in that window left the event anchored on the home R had just left, after that
//! home's `resource_departed` — so the home's readers saw R still live there, and a departure,
//! which is built to read the same as a deletion (migration `20261117000040`), read as a move. And
//! the authority the doors decided against the old home outlived it.
//!
//! **In flight:** each race holds a move of R in a transaction left open, past its home update.
//! The reassignment is started and the test waits until it is blocked on that transaction's
//! backend (`pg_stat_activity`, `pg_blocking_pids`). With the fix it blocks on R's writer lock;
//! without it, on the move's home row, after it has read the old home — both are a wait on the
//! holder, so the wait proves the reassignment was in flight, not that it ran after the commit.
//! The holder commits, and the reassignment's outcome is read.
//!
//! Every fixture is a plain row except the move, which runs the real write
//! (`writes::update_resource_in_tx` with `rehome_to`).

use sqlx::PgPool;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_services::error::ApiError;
use temper_services::services::reassign_service;
use temper_services::test_support;
use temper_substrate::events::EventContext;
use temper_substrate::writes::{self, UpdateParams};
use uuid::Uuid;

// ── Fixture ───────────────────────────────────────────────────────────────────────────────────────

async fn profile(pool: &PgPool, handle: &str) -> Uuid {
    let id = temper_services::test_support::profile::insert(pool, handle).await;
    temper_services::test_support::entity::insert(pool, id, &format!("{handle}@web")).await;
    id
}

async fn context(pool: &PgPool, slug: &str, owner_table: &str, owner: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
         VALUES ($1, $1, $2, $3) RETURNING id",
    )
    .bind(slug)
    .bind(owner_table)
    .bind(owner)
    .fetch_one(pool)
    .await
    .expect("seed context")
}

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("seed team")
}

async fn member(pool: &PgPool, team: Uuid, who: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team)
    .bind(who)
    .bind(role)
    .execute(pool)
    .await
    .expect("seed member");
}

async fn resource_in(pool: &PgPool, ctx: Uuid, owner: Uuid) -> Uuid {
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('r', 'test://r') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("seed resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(rid)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .expect("seed home");
    rid
}

/// Move `resource` to `dest` as `mover` in a transaction left open past the move's home update,
/// and answer it with its backend pid.
async fn hold_move(
    pool: &PgPool,
    resource: Uuid,
    dest: Uuid,
    mover: Uuid,
) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let emitter = writes::resolve_emitter(pool, ProfileId::from(mover), "web")
        .await
        .expect("mover's emitter");
    let mut held = pool.begin().await.expect("begin the move");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *held)
        .await
        .expect("holder pid");
    let outcome = writes::update_resource_in_tx(
        &mut held,
        UpdateParams {
            resource: ResourceId::from(resource),
            body: None,
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &[],
            chunks: None,
            sources: Vec::new(),
            content_block: None,
            rehome_to: Some(ContextId::from(dest)),
            emitter,
        },
        EventContext::default(),
        false,
    )
    .await
    .expect("the move");
    assert!(
        outcome.left_home.is_some(),
        "the held update must be a move"
    );
    (held, pid)
}

/// Wait until a backend in this test's database is blocked on a lock `holder` holds. Panics if
/// `request` finishes first (it never waited on the holder) or nothing waits within 10 seconds.
async fn blocked_on<T>(pool: &PgPool, holder: i32, request: &tokio::task::JoinHandle<T>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        assert!(
            !request.is_finished(),
            "the reassignment finished while the move was held — it never waited on the move"
        );
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)))",
        )
        .bind(holder)
        .fetch_one(pool)
        .await
        .expect("read lock waiters");
        if waiting {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the reassignment never blocked on the held move"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

async fn owner_of(pool: &PgPool, resource: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT owner_profile_id FROM kb_resource_homes WHERE resource_id = $1")
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("owner")
}

/// The anchors of `resource`'s `resource_reassigned` events, in record order.
async fn reassigned_anchors(pool: &PgPool, resource: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT e.producing_anchor_id FROM kb_events e \
           JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_reassigned' \
            AND (e.payload->>'resource_id')::uuid = $1 \
          ORDER BY e.xact_id, e.seq",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .expect("reassigned events")
}

// ── Witnesses ─────────────────────────────────────────────────────────────────────────────────────

/// **The event anchors on the home R has when it is written.** The owner reassigns R while a move
/// of R from A to B is in flight. The reassignment lands, and its `resource_reassigned` is anchored
/// on B — never on A, which R left.
///
/// FAILS IF the reassignment does not take R's writer lock before `resource_reassign` reads the
/// home: it reads A, waits on the move's home row, and anchors on A. The bite: delete the
/// `lock_resource_for_write` calls in `writes::reassign_resource_in_tx` and in
/// `reassign_service::reassign_resource`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_reassignment_racing_a_move_anchors_on_the_home_it_moved_to(pool: PgPool) {
    let owner = profile(&pool, "race-owner").await;
    let to = profile(&pool, "race-to").await;
    let a = context(&pool, "race-a", "kb_profiles", owner).await;
    let b = context(&pool, "race-b", "kb_profiles", owner).await;
    let r = resource_in(&pool, a, owner).await;
    let caller = test_support::human_principal_for(&pool, owner).await;

    let (held, holder) = hold_move(&pool, r, b, owner).await;
    let p = pool.clone();
    let request =
        tokio::spawn(async move { reassign_service::reassign_resource(&p, &caller, r, to).await });
    blocked_on(&pool, holder, &request).await;
    held.commit().await.expect("commit the move");
    request
        .await
        .expect("the reassignment must not panic")
        .expect("the owner's reassignment lands");

    assert_eq!(owner_of(&pool, r).await, to, "the reassignment landed");
    assert_eq!(
        reassigned_anchors(&pool, r).await,
        vec![b],
        "resource_reassigned must anchor on the home R moved to, not the one it left"
    );
}

/// **Authority is decided against the home R is in when the event is written.** A team maintainer
/// reassigns R, homed in the team's context A, while its owner moves R into their own context B.
/// The maintainer's reach came through A; over B they have none, so the reassignment is refused as
/// a stranger's is, and R keeps its owner.
///
/// FAILS IF the single door does not re-decide its authority under R's writer lock: the reach read
/// on the pool admits the maintainer, and R in B is handed to them. The bite: drop the
/// `authorize_single` call under the lock in `reassign_service::reassign_resource` and write with
/// the home the unlocked pre-check authorized. (A fresh pool read taken after the lock is no bite:
/// it too sees the committed move.)
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_admin_whose_reach_a_move_takes_away_is_refused(pool: PgPool) {
    let owner = profile(&pool, "race-member").await;
    let admin = profile(&pool, "race-admin").await;
    let t = team(&pool, "race-team").await;
    member(&pool, t, owner, "member").await;
    member(&pool, t, admin, "maintainer").await;
    let a = context(&pool, "race-team-ctx", "kb_teams", t).await;
    let b = context(&pool, "race-own-ctx", "kb_profiles", owner).await;
    let r = resource_in(&pool, a, owner).await;
    let caller = test_support::human_principal_for(&pool, admin).await;

    let (held, holder) = hold_move(&pool, r, b, owner).await;
    let p = pool.clone();
    let request =
        tokio::spawn(
            async move { reassign_service::reassign_resource(&p, &caller, r, admin).await },
        );
    blocked_on(&pool, holder, &request).await;
    held.commit().await.expect("commit the move");
    let answer = request.await.expect("the reassignment must not panic");

    assert!(
        matches!(answer, Err(ApiError::Forbidden)),
        "an admin with no reach over R's new home is refused, got {answer:?}"
    );
    assert_eq!(owner_of(&pool, r).await, owner, "R keeps its owner");
    assert!(
        reassigned_anchors(&pool, r).await.is_empty(),
        "no reassignment was recorded"
    );
}

/// **A team handoff skips a resource moved out of its scope in flight.** A maintainer hands off a
/// member's work while the member moves R from the team's context A into their own context B. R
/// read as in scope on the pool; under its writer lock it is not, so the run skips it: R keeps its
/// owner, is not among the returned ids, and no reassignment of it is recorded.
///
/// FAILS IF the bulk door does not re-check the scope under R's writer lock: R in B is handed off,
/// its event anchored on B (or, without the lock at all, on A). The bite: delete the
/// `still_in_team_scope` check in `reassign_service::reassign_team_resources`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_team_handoff_skips_a_resource_moved_out_of_its_scope(pool: PgPool) {
    let leaver = profile(&pool, "race-leaver").await;
    let admin = profile(&pool, "race-handoff-admin").await;
    let heir = profile(&pool, "race-heir").await;
    let t = team(&pool, "race-handoff").await;
    member(&pool, t, leaver, "member").await;
    member(&pool, t, admin, "maintainer").await;
    member(&pool, t, heir, "member").await;
    let a = context(&pool, "race-handoff-ctx", "kb_teams", t).await;
    let b = context(&pool, "race-leaver-ctx", "kb_profiles", leaver).await;
    let r = resource_in(&pool, a, leaver).await;
    let caller = test_support::human_principal_for(&pool, admin).await;

    let (held, holder) = hold_move(&pool, r, b, leaver).await;
    let p = pool.clone();
    let request = tokio::spawn(async move {
        reassign_service::reassign_team_resources(&p, &caller, t, leaver, heir).await
    });
    blocked_on(&pool, holder, &request).await;
    held.commit().await.expect("commit the move");
    let moved = request
        .await
        .expect("the handoff must not panic")
        .expect("the handoff runs");

    assert!(
        moved.is_empty(),
        "R left the scope, so it is not handed off: {moved:?}"
    );
    assert_eq!(owner_of(&pool, r).await, leaver, "R keeps its owner");
    assert!(
        reassigned_anchors(&pool, r).await.is_empty(),
        "no reassignment was recorded"
    );
}
