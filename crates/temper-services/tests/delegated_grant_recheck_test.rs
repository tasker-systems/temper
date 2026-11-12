#![cfg(feature = "test-db")]
//! The grant door's delegated arm decided the granter's grant right on a pool read
//! (`GrantAuthority::resolve`'s `can(grant)`) and never asked again on its write. A grant outlives
//! the right that made it, so a granter whose `can_grant` came through a team, and who was removed
//! from that team or had the team's right withdrawn while the door was in flight, still wrote the
//! grant. The door now asks again on its transaction: the teams the right is held through under
//! their row locks (`team_membership::role_under_lock`), then the grant rows carrying it
//! `FOR SHARE`.
//!
//! **In flight:** the removal or withdrawal runs in a transaction left open, the grant is started,
//! and the test waits until the grant is blocked on that transaction's backend at the re-check's
//! statement. The pool reads ahead of the re-check are plain and never wait on an uncommitted
//! write, so a block there can only be the re-check. The holder commits; the grant is refused as a
//! stranger's is, and no grant row was written. Without the re-check the grant never blocks and the
//! wait times out.

use sqlx::PgPool;
use temper_core::types::cognitive_maps::GrantCapabilityRequest;
use temper_core::types::team::TeamRole;
use temper_services::error::ApiError;
use temper_services::services::access_service;
use temper_services::test_support;
use temper_workflow::operations::Surface;
use uuid::Uuid;

// ── Fixture ───────────────────────────────────────────────────────────────────────────────────────

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

async fn member(pool: &PgPool, team: Uuid, who: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team)
    .bind(who)
    .bind(role)
    .execute(pool)
    .await
    .expect("member");
}

/// The cast of every race: a context owned by a team the granter has nothing to do with; a team
/// `source` the granter maintains, holding read + grant on the context; a team `to` the granter
/// owns, the grant's recipient; and a stranger who owns a team of their own and holds no right.
struct Fixture {
    admin: Uuid,
    granter: Uuid,
    stranger: Uuid,
    source: Uuid,
    to: Uuid,
    stranger_team: Uuid,
    ctx: Uuid,
}

async fn fixture(pool: &PgPool) -> Fixture {
    let admin = profile(pool, "dg-admin").await;
    test_support::approved_admin(pool, admin).await;
    let granter = profile(pool, "dg-granter").await;
    let stranger = profile(pool, "dg-stranger").await;
    let source_owner = profile(pool, "dg-source-owner").await;
    let owning = team(pool, "dg-owning").await;
    let source = team(pool, "dg-source").await;
    let to = team(pool, "dg-to").await;
    let stranger_team = team(pool, "dg-stranger-team").await;
    member(pool, source, source_owner, "owner").await;
    member(pool, source, granter, "maintainer").await;
    member(pool, to, granter, "owner").await;
    member(pool, stranger_team, stranger, "owner").await;
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_teams', $1, 'dg-ctx', 'dg-ctx') RETURNING id",
    )
    .bind(owning)
    .fetch_one(pool)
    .await
    .expect("context");
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, can_delete, can_grant, \
                                       granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_teams', $2, true, false, false, true, $3)",
    )
    .bind(ctx)
    .bind(source)
    .bind(admin)
    .execute(pool)
    .await
    .expect("the source team's grant right");
    Fixture {
        admin,
        granter,
        stranger,
        source,
        to,
        stranger_team,
        ctx,
    }
}

fn read_on(ctx: Uuid, to: Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_contexts".into(),
        subject_id: ctx,
        principal_table: "kb_teams".into(),
        principal_id: to,
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
    }
}

async fn grant_as(pool: &PgPool, who: Uuid, req: &GrantCapabilityRequest) -> Result<(), ApiError> {
    let by = test_support::human_principal_for(pool, who).await;
    access_service::grant_capability(pool, &by, req)
        .await
        .map(|_| ())
}

async fn grant_rows_to(pool: &PgPool, subject: Uuid, team: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants WHERE subject_id = $1 \
           AND principal_table = 'kb_teams' AND principal_id = $2",
    )
    .bind(subject)
    .bind(team)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// What a caller with no grant right on the context is told.
async fn strangers_refusal(pool: &PgPool, f: &Fixture) -> String {
    let err = grant_as(pool, f.stranger, &read_on(f.ctx, f.stranger_team))
        .await
        .expect_err("a stranger confers nothing");
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    format!("{err:?}")
}

/// Wait until a backend is blocked on a lock `holder` holds, at a statement matching `statement`.
async fn await_blocked_at(pool: &PgPool, holder: i32, statement: &str) {
    await_n_blocked_at(pool, holder, statement, 1).await;
}

/// Wait until at least `n` backends are blocked at `statement` while `holder` is open. With `n > 1`
/// a later waiter queues behind an earlier one (Postgres hands a row lock on in queue order, so its
/// blocking pid is the earlier waiter, not `holder`): the count is of every waiter at the
/// statement, and the first must be waiting on `holder`.
async fn await_n_blocked_at(pool: &PgPool, holder: i32, statement: &str, n: i64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT CASE WHEN EXISTS (SELECT 1 FROM pg_stat_activity \
                                       WHERE datname = current_database() \
                                         AND wait_event_type = 'Lock' AND query ILIKE $2 \
                                         AND $1 = ANY (pg_blocking_pids(pid))) \
                         THEN (SELECT count(*) FROM pg_stat_activity \
                                WHERE datname = current_database() \
                                  AND wait_event_type = 'Lock' AND query ILIKE $2) \
                         ELSE 0 END",
        )
        .bind(holder)
        .bind(statement)
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting >= n {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{waiting} of {n} grants blocked at `{statement}`: the re-check did not wait on the lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

async fn backend_pid(conn: &mut sqlx::PgConnection) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .expect("pid")
}

// ── The witnesses ─────────────────────────────────────────────────────────────────────────────────

/// BITE: the granter is removed from the team their grant right comes through, uncommitted. The
/// grant blocks on that team's row; the removal commits; the grant is refused as a stranger's is,
/// and the recipient holds nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_granter_removed_from_the_source_team_in_flight_writes_no_grant(pool: PgPool) {
    let f = fixture(&pool).await;
    let stranger = strangers_refusal(&pool, &f).await;

    let mut holder = test_support::hold_member_removal(&pool, f.admin, f.source, f.granter).await;
    let pid = backend_pid(&mut holder).await;
    let (p, who, req) = (pool.clone(), f.granter, read_on(f.ctx, f.to));
    let contender = tokio::spawn(async move { grant_as(&p, who, &req).await });
    await_blocked_at(&pool, pid, "%FROM kb_teams%FOR NO KEY UPDATE%").await;
    holder.commit().await.unwrap();

    let err = contender
        .await
        .unwrap()
        .expect_err("a granter removed under the lock confers nothing");
    assert_eq!(format!("{err:?}"), stranger, "told what a stranger is told");
    assert_eq!(grant_rows_to(&pool, f.ctx, f.to).await, 0, "no grant row");
}

/// BITE: the source team's grant right is withdrawn (`can_grant = false`), uncommitted. The grant
/// blocks on that grant row; the withdrawal commits; the grant is refused as a stranger's is, and
/// the recipient holds nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_grant_right_withdrawn_from_the_source_team_in_flight_writes_no_grant(pool: PgPool) {
    let f = fixture(&pool).await;
    let stranger = strangers_refusal(&pool, &f).await;

    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    sqlx::query(
        "UPDATE kb_access_grants SET can_grant = false \
          WHERE subject_id = $1 AND principal_table = 'kb_teams' AND principal_id = $2",
    )
    .bind(f.ctx)
    .bind(f.source)
    .execute(&mut *holder)
    .await
    .unwrap();
    let (p, who, req) = (pool.clone(), f.granter, read_on(f.ctx, f.to));
    let contender = tokio::spawn(async move { grant_as(&p, who, &req).await });
    await_blocked_at(&pool, pid, "%FROM kb_access_grants%FOR SHARE%").await;
    holder.commit().await.unwrap();

    let err = contender
        .await
        .unwrap()
        .expect_err("a grant right withdrawn under the lock confers nothing");
    assert_eq!(format!("{err:?}"), stranger, "told what a stranger is told");
    assert_eq!(grant_rows_to(&pool, f.ctx, f.to).await, 0, "no grant row");
}

/// BITE: the granter is demoted on the source team from maintainer to member through the routed
/// membership writer, uncommitted. A member holds no team-anchored grant right. The grant blocks on
/// the team row; the demotion commits; the grant is refused as a stranger's is.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_granter_demoted_on_the_source_team_in_flight_writes_no_grant(pool: PgPool) {
    let f = fixture(&pool).await;
    let stranger = strangers_refusal(&pool, &f).await;

    let mut holder = test_support::hold_member_role_change(
        &pool,
        f.admin,
        f.source,
        f.granter,
        TeamRole::Member,
    )
    .await;
    let pid = backend_pid(&mut holder).await;
    let (p, who, req) = (pool.clone(), f.granter, read_on(f.ctx, f.to));
    let contender = tokio::spawn(async move { grant_as(&p, who, &req).await });
    await_blocked_at(&pool, pid, "%FROM kb_teams%FOR NO KEY UPDATE%").await;
    holder.commit().await.unwrap();

    let err = contender
        .await
        .unwrap()
        .expect_err("a granter demoted under the lock confers nothing");
    assert_eq!(format!("{err:?}"), stranger, "told what a stranger is told");
    assert_eq!(grant_rows_to(&pool, f.ctx, f.to).await, 0, "no grant row");
}

/// BITE: the granter is removed from the RECIPIENT team, uncommitted. The grant's manage bar on
/// the team it grants to was a pool read; the grant now locks that team in its ascending pass and
/// asks the bar again. It blocks on the team row; the removal commits; the grant is refused as a
/// stranger's is, and the team holds nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_granter_removed_from_the_recipient_team_in_flight_writes_no_grant(pool: PgPool) {
    let f = fixture(&pool).await;
    let stranger = strangers_refusal(&pool, &f).await;
    let co_owner = profile(&pool, "dg-to-co-owner").await;
    member(&pool, f.to, co_owner, "owner").await;

    let mut holder = test_support::hold_member_removal(&pool, f.admin, f.to, f.granter).await;
    let pid = backend_pid(&mut holder).await;
    let (p, who, req) = (pool.clone(), f.granter, read_on(f.ctx, f.to));
    let contender = tokio::spawn(async move { grant_as(&p, who, &req).await });
    await_blocked_at(&pool, pid, "%FROM kb_teams%FOR NO KEY UPDATE%").await;
    holder.commit().await.unwrap();

    let err = contender
        .await
        .unwrap()
        .expect_err("a granter removed from the recipient team confers nothing on it");
    assert_eq!(format!("{err:?}"), stranger, "told what a stranger is told");
    assert_eq!(grant_rows_to(&pool, f.ctx, f.to).await, 0, "no grant row");
}

/// BITE: a grant to the granter's own machine needs a grant right held in their own name. The
/// granter holds one in person and one through the source team; the in-person right is withdrawn,
/// uncommitted. The pool check saw it, so the grant reaches the re-check and blocks on that row;
/// the withdrawal commits. Only the team-held right is left, which does not carry the grant to a
/// machine the granter controls: refused as a stranger's is, and the machine holds nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_in_person_right_withdrawn_in_flight_grants_nothing_to_the_granters_machine(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let stranger = strangers_refusal(&pool, &f).await;
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, can_delete, can_grant, \
                                       granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, false, false, true, $3)",
    )
    .bind(f.ctx)
    .bind(f.granter)
    .bind(f.admin)
    .execute(&pool)
    .await
    .expect("the in-person right");
    let machine = profile(&pool, "dg-granters-machine").await;
    sqlx::query(
        "INSERT INTO kb_machine_clients \
           (client_id, issuer, label, profile_id, team_id, registered_by_profile_id) \
         VALUES ('dg-machine', 'auth0-m2m', 'dg', $1, NULL, $2)",
    )
    .bind(machine)
    .bind(f.granter)
    .execute(&pool)
    .await
    .expect("the granter's machine");
    let to_machine = GrantCapabilityRequest {
        principal_table: "kb_profiles".into(),
        principal_id: machine,
        ..read_on(f.ctx, f.to)
    };

    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    sqlx::query(
        "UPDATE kb_access_grants SET can_grant = false \
          WHERE subject_id = $1 AND principal_table = 'kb_profiles' AND principal_id = $2",
    )
    .bind(f.ctx)
    .bind(f.granter)
    .execute(&mut *holder)
    .await
    .unwrap();
    let (p, who) = (pool.clone(), f.granter);
    let contender = tokio::spawn(async move { grant_as(&p, who, &to_machine).await });
    await_blocked_at(&pool, pid, "%FROM kb_access_grants%FOR SHARE%").await;
    holder.commit().await.unwrap();

    let err = contender
        .await
        .unwrap()
        .expect_err("a team-held right confers nothing on the granter's own machine");
    assert_eq!(format!("{err:?}"), stranger, "told what a stranger is told");
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants WHERE subject_id = $1 AND principal_id = $2",
    )
    .bind(f.ctx)
    .bind(machine)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 0, "no grant row on the machine");
}

// ── Pins ──────────────────────────────────────────────────────────────────────────────────────────

/// PIN: the re-check admits who the pool admitted. A team-held right still grants, and a delegate
/// may rewrite the very row their right comes from: it is share-locked by the re-check and then
/// taken `FOR UPDATE` by the write.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_team_held_right_still_grants_and_may_rewrite_its_own_row(pool: PgPool) {
    let f = fixture(&pool).await;
    grant_as(&pool, f.granter, &read_on(f.ctx, f.to))
        .await
        .expect("a delegate with a team-held right grants");
    assert_eq!(grant_rows_to(&pool, f.ctx, f.to).await, 1);

    let own_row = GrantCapabilityRequest {
        can_grant: true,
        ..read_on(f.ctx, f.source)
    };
    grant_as(&pool, f.granter, &own_row)
        .await
        .expect("a manager of the source team rewrites its row");
    assert_eq!(grant_rows_to(&pool, f.ctx, f.source).await, 1);
}

/// BITE: two managers of the source team rewriting its row at once both land. Each takes the
/// source team's row lock before it share-locks the grant row and then takes it `FOR UPDATE`, so the
/// two are serialized at the team row and the share-then-update never interleaves (interleaved, the
/// two would each hold the row shared and deadlock on the upgrade). A holder takes the team row
/// first; both rewrites must be seen waiting on it before it lets go.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn two_managers_rewriting_the_source_row_at_once_both_land(pool: PgPool) {
    let f = fixture(&pool).await;
    let second = profile(&pool, "dg-second").await;
    member(&pool, f.source, second, "maintainer").await;
    let own_row = GrantCapabilityRequest {
        can_grant: true,
        ..read_on(f.ctx, f.source)
    };

    let mut holder = pool.begin().await.unwrap();
    let pid = backend_pid(&mut holder).await;
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(f.source)
        .execute(&mut *holder)
        .await
        .unwrap();
    let (p1, r1, g1) = (pool.clone(), own_row.clone(), f.granter);
    let (p2, r2) = (pool.clone(), own_row.clone());
    let a = tokio::spawn(async move { grant_as(&p1, g1, &r1).await });
    let b = tokio::spawn(async move { grant_as(&p2, second, &r2).await });
    await_n_blocked_at(&pool, pid, "%FROM kb_teams%FOR NO KEY UPDATE%", 2).await;
    holder.commit().await.unwrap();

    a.await.unwrap().expect("the first rewrite");
    b.await.unwrap().expect("the second rewrite");
}

/// PIN: a granter who holds the right both in person and through the source team keeps it when the
/// team's is gone: the re-check counts any carrying row, not the one the pool happened to see.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_right_also_held_in_person_survives_losing_the_team(pool: PgPool) {
    let f = fixture(&pool).await;
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, can_delete, can_grant, \
                                       granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, false, false, true, $3)",
    )
    .bind(f.ctx)
    .bind(f.granter)
    .bind(f.admin)
    .execute(&pool)
    .await
    .expect("the in-person right");
    test_support::hold_member_removal(&pool, f.admin, f.source, f.granter)
        .await
        .commit()
        .await
        .unwrap();
    grant_as(&pool, f.granter, &read_on(f.ctx, f.to))
        .await
        .expect("the in-person right still grants");
    assert_eq!(grant_rows_to(&pool, f.ctx, f.to).await, 1);
}
