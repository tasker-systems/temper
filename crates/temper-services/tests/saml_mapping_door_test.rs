#![cfg(feature = "test-db")]
//! The SAML group-mapping doors (`saml_mapping_service`): create and remove a
//! `kb_saml_group_mappings` row as a system admin's recorded act.
//!
//! Before these doors the only writer was SQL the CLI rendered for psql, with no ceiling, no
//! removal and no record. The witnesses here:
//!
//! - an `owner` mapping, and a mapping onto temper-system or the gating team, are refused without
//!   `confirm_elevated` and write nothing (the rendered SQL wrote any role onto any team);
//! - with confirmation each succeeds and its record says it was confirmed;
//! - create and remove each append exactly one `saml_group_mapping_changed` event naming the actor,
//!   the IdP, the group, the team and the role, and a refused or repeated act appends none;
//! - removal leaves the team's memberships in place and its answer says so;
//! - both acts re-ask the admin's standing under the governance lock, so a demotion in flight
//!   refuses them with nothing written.

use sqlx::PgPool;
use temper_core::types::saml_mapping::{
    CreateSamlGroupMappingRequest, RemoveSamlGroupMappingParams, MAPPING_REMOVAL_LEAVES_MEMBERSHIPS,
};
use temper_core::types::team::TeamRole;
use temper_services::error::ApiError;
use temper_services::services::saml_mapping_service;
use temper_services::test_support;
use temper_workflow::operations::Surface;
use uuid::Uuid;

const IDP: &str = "sm-okta";

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
    sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) \
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("team")
}

struct Fixture {
    admin: Uuid,
    stranger: Uuid,
    eng: Uuid,
    root: Uuid,
    gating: Uuid,
}

async fn fixture(pool: &PgPool) -> Fixture {
    let admin = profile(pool, "sm-admin").await;
    test_support::approved_admin(pool, admin).await;
    let stranger = profile(pool, "sm-stranger").await;
    test_support::approve(pool, stranger).await;
    let eng = team(pool, "sm-eng").await;
    let root = team(pool, "temper-system").await;
    let gating = team(pool, "sm-gate").await;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'sm-gate' WHERE id = 1")
        .execute(pool)
        .await
        .expect("point the gating slug");
    sqlx::query(
        "INSERT INTO kb_saml_idp (idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id, \
                                  sp_entity_id, acs_url, nameid_format, email_attr, \
                                  stable_id_attr, groups_attr) \
         VALUES ($1, true, 'x', 'https://idp/sso', 'idp', 'sp', 'https://sp/acs', 'persistent', \
                 'email', 'uid', 'groups')",
    )
    .bind(IDP)
    .execute(pool)
    .await
    .expect("seed idp");
    Fixture {
        admin,
        stranger,
        eng,
        root,
        gating,
    }
}

fn req(team_id: Uuid, group: &str, role: TeamRole, confirm: bool) -> CreateSamlGroupMappingRequest {
    CreateSamlGroupMappingRequest {
        idp_key: IDP.to_string(),
        group: group.to_string(),
        team_id,
        role,
        confirm_elevated: confirm,
    }
}

fn key(team_id: Uuid, group: &str) -> RemoveSamlGroupMappingParams {
    RemoveSamlGroupMappingParams {
        idp_key: IDP.to_string(),
        group: group.to_string(),
        team_id,
    }
}

/// Every mapping row, as `(group, team, role)`.
async fn mappings(pool: &PgPool) -> Vec<(String, Uuid, String)> {
    sqlx::query_as(
        "SELECT group_value, team_id, role::text FROM kb_saml_group_mappings \
          ORDER BY group_value, team_id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Every `saml_group_mapping_changed` payload, oldest first.
async fn records(pool: &PgPool) -> Vec<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'saml_group_mapping_changed' ORDER BY e.id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

fn assert_needs_confirmation(err: ApiError, why: &str) {
    match err {
        ApiError::BadRequest(m) => {
            assert!(m.contains("confirmation required"), "{m}");
            assert!(m.contains("confirm_elevated"), "{m}");
            assert!(m.contains(why), "{m} should name {why}");
        }
        other => panic!("expected a named confirmation refusal, got {other:?}"),
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_owner_mapping_without_confirmation_is_refused_and_writes_nothing(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;

    let err = saml_mapping_service::create(
        &pool,
        &admin,
        &req(f.eng, "eng-leads", TeamRole::Owner, false),
    )
    .await
    .unwrap_err();
    assert_needs_confirmation(err, "`owner` is above the `maintainer` ceiling");
    assert!(
        mappings(&pool).await.is_empty(),
        "a refused create wrote a row"
    );
    assert!(records(&pool).await.is_empty(), "a refused create recorded");

    // `maintainer` is the ceiling: it needs no confirmation.
    let out = saml_mapping_service::create(
        &pool,
        &admin,
        &req(f.eng, "eng", TeamRole::Maintainer, false),
    )
    .await
    .expect("maintainer is within the ceiling");
    assert!(out.created && !out.confirmed);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_instance_team_target_without_confirmation_is_refused(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;

    for target in [f.root, f.gating] {
        let err = saml_mapping_service::create(
            &pool,
            &admin,
            &req(target, "everyone", TeamRole::Member, false),
        )
        .await
        .unwrap_err();
        assert_needs_confirmation(err, "instance team");
    }
    // Both conditions at once are both named.
    let err = saml_mapping_service::create(
        &pool,
        &admin,
        &req(f.root, "admins", TeamRole::Owner, false),
    )
    .await
    .unwrap_err();
    let msg = format!("{err:?}");
    assert_needs_confirmation(err, "`owner`");
    assert!(
        msg.contains("instance team"),
        "both conditions are named: {msg}"
    );
    assert!(mappings(&pool).await.is_empty());
    assert!(records(&pool).await.is_empty());
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn with_confirmation_both_succeed_and_are_recorded_as_confirmed(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;

    let owner = saml_mapping_service::create(
        &pool,
        &admin,
        &req(f.eng, "eng-leads", TeamRole::Owner, true),
    )
    .await
    .expect("a confirmed owner mapping");
    assert!(owner.created && owner.confirmed);
    let gate = saml_mapping_service::create(
        &pool,
        &admin,
        &req(f.gating, "staff", TeamRole::Member, true),
    )
    .await
    .expect("a confirmed gating-team mapping");
    assert!(gate.created && gate.confirmed);
    // Confirmation on an ordinary mapping is ignored, and not recorded as one.
    let plain =
        saml_mapping_service::create(&pool, &admin, &req(f.eng, "eng", TeamRole::Member, true))
            .await
            .unwrap();
    assert!(plain.created && !plain.confirmed);

    let recs = records(&pool).await;
    assert_eq!(recs.len(), 3);
    assert_eq!(recs[0]["confirmed"], true);
    assert_eq!(recs[1]["confirmed"], true);
    assert!(recs[2].get("confirmed").is_none(), "{}", recs[2]);
    assert_eq!(mappings(&pool).await.len(), 3);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn create_and_remove_each_record_one_event_with_the_actor_group_team_and_role(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;

    saml_mapping_service::create(
        &pool,
        &admin,
        &req(f.eng, "eng", TeamRole::Maintainer, false),
    )
    .await
    .unwrap();
    let recs = records(&pool).await;
    assert_eq!(recs.len(), 1, "one create, one record");
    let want = |act: &str| {
        serde_json::json!({
            "act": act,
            "actor": f.admin,
            "idp_key": IDP,
            "group_value": "eng",
            "team_id": f.eng,
            "role": "maintainer",
        })
    };
    assert_eq!(recs[0], want("created"));

    // The same mapping again changes nothing and records nothing.
    let again = saml_mapping_service::create(
        &pool,
        &admin,
        &req(f.eng, "eng", TeamRole::Maintainer, false),
    )
    .await
    .unwrap();
    assert!(!again.created);
    // The same key at another role is a named conflict, and records nothing.
    let err =
        saml_mapping_service::create(&pool, &admin, &req(f.eng, "eng", TeamRole::Member, false))
            .await
            .unwrap_err();
    assert!(
        matches!(&err, ApiError::Conflict(m) if m.contains("`maintainer`")),
        "{err:?}"
    );
    assert_eq!(records(&pool).await.len(), 1);

    saml_mapping_service::remove(&pool, &admin, &key(f.eng, "eng"))
        .await
        .unwrap();
    let recs = records(&pool).await;
    assert_eq!(recs.len(), 2, "one remove, one record");
    assert_eq!(recs[1], want("removed"));
    assert!(mappings(&pool).await.is_empty());

    // Removing what is not there is a 404 and records nothing.
    let err = saml_mapping_service::remove(&pool, &admin, &key(f.eng, "eng"))
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::NotFound(_)), "{err:?}");
    assert_eq!(records(&pool).await.len(), 2);

    // The record carries the team as its subject reference, the axis the ledger reads by team.
    let subjects: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
         WHERE t.name = 'saml_group_mapping_changed' \
           AND e.\"references\" @> jsonb_build_array(jsonb_build_object('rel', 'subject', \
                 'target', jsonb_build_object('kind', 'kb_teams', 'id', $1::uuid)))",
    )
    .bind(f.eng)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(subjects, 2);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn removal_leaves_memberships_in_place_and_says_so(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;
    saml_mapping_service::create(&pool, &admin, &req(f.eng, "eng", TeamRole::Member, false))
        .await
        .unwrap();
    // A membership the reconcile wrote from this mapping.
    let alice = profile(&pool, "sm-alice").await;
    test_support::approve(&pool, alice).await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
         VALUES ($1, $2, 'member', 'idp')",
    )
    .bind(f.eng)
    .bind(alice)
    .execute(&pool)
    .await
    .unwrap();

    let out = saml_mapping_service::remove(&pool, &admin, &key(f.eng, "eng"))
        .await
        .unwrap();
    assert_eq!(out.removed.role, TeamRole::Member);
    assert_eq!(out.idp_memberships_left_in_place, 1);
    assert_eq!(out.memberships, MAPPING_REMOVAL_LEAVES_MEMBERSHIPS);
    let still: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(f.eng)
    .bind(alice)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(still, 1, "the removal deleted a membership");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_non_admin_cannot_mint_the_proof_the_doors_take(pool: PgPool) {
    let f = fixture(&pool).await;
    let human = test_support::human_principal_for(&pool, f.stranger).await;
    let err = temper_services::auth::require_system_admin(&pool, &human)
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn unknown_idp_dead_team_and_personal_team_are_refused(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;

    let mut unknown = req(f.eng, "eng", TeamRole::Member, false);
    unknown.idp_key = "no-such-idp".into();
    let err = saml_mapping_service::create(&pool, &admin, &unknown)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ApiError::NotFound(m) if m.contains("no-such-idp")),
        "{err:?}"
    );

    let dead = team(&pool, "sm-dead").await;
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(dead)
        .execute(&pool)
        .await
        .unwrap();
    let err =
        saml_mapping_service::create(&pool, &admin, &req(dead, "eng", TeamRole::Member, false))
            .await
            .unwrap_err();
    assert!(matches!(err, ApiError::NotFound(_)), "{err:?}");

    let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(f.stranger)
        .fetch_one(&pool)
        .await
        .expect("the stranger's personal team");
    let err = saml_mapping_service::create(
        &pool,
        &admin,
        &req(personal, "eng", TeamRole::Member, false),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, ApiError::BadRequest(m) if m.contains("personal team")),
        "{err:?}"
    );
    assert!(mappings(&pool).await.is_empty());
    assert!(records(&pool).await.is_empty());
}

// ── The governance re-check ─────────────────────────────────────────────────────────────────────

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

/// A proof minted before a demotion in flight: the door waits at the re-check, the demotion
/// commits, and the door is refused with nothing written.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_demotion_in_flight_refuses_create_and_remove(pool: PgPool) {
    let f = fixture(&pool).await;

    // Create.
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;
    let (holder, pid) = demotion_in_flight(&pool, f.admin).await;
    let contender = tokio::spawn({
        let pool = pool.clone();
        let r = req(f.eng, "eng", TeamRole::Member, false);
        async move { saml_mapping_service::create(&pool, &admin, &r).await }
    });
    await_blocked_at_the_recheck(&pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert!(mappings(&pool).await.is_empty(), "a refused create wrote");
    assert!(records(&pool).await.is_empty(), "a refused create recorded");

    // Remove: restore the grant, make a mapping, then race its removal.
    test_support::grant_governance(&pool, f.admin).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;
    saml_mapping_service::create(&pool, &admin, &req(f.eng, "eng", TeamRole::Member, false))
        .await
        .unwrap();
    let (holder, pid) = demotion_in_flight(&pool, f.admin).await;
    let contender = tokio::spawn({
        let pool = pool.clone();
        let k = key(f.eng, "eng");
        async move { saml_mapping_service::remove(&pool, &admin, &k).await }
    });
    await_blocked_at_the_recheck(&pool, pid).await;
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    assert_eq!(mappings(&pool).await.len(), 1, "a refused remove deleted");
    assert_eq!(records(&pool).await.len(), 1, "a refused remove recorded");
}

// ── The migration's refusal over existing rows (Pete, 2026-11-11) ────────────────────────────────

/// Migration 20261125000010, read whole so its Section 0 guard can run against rows it must refuse.
const DOOR_MIGRATION: &str =
    include_str!("../../../migrations/20261125000010_saml_group_mapping_door.sql");

/// FAILS IF the migration would apply over a mapping that is `owner` or targets temper-system or the
/// gating team, or stops naming it. A mapping within the ceiling on an ordinary team passes.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_migration_refuses_an_unconfirmed_mapping_above_the_ceiling(pool: PgPool) {
    use sqlx::Executor;
    let section = DOOR_MIGRATION
        .split_once("-- Section 0.")
        .and_then(|(_, rest)| rest.split_once("-- Section 1."))
        .map(|(body, _)| body)
        .expect("the migration carries its Section 0 guard");
    let guard = &section[section.find("DO $$").expect("the guard is a DO block")..];
    let f = fixture(&pool).await;

    let insert = |team: Uuid, group: &'static str, role: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) \
                 VALUES ($1, $2, $3, $4::team_role)",
            )
            .bind(IDP)
            .bind(group)
            .bind(team)
            .bind(role)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    let clear = || async {
        sqlx::query("DELETE FROM kb_saml_group_mappings")
            .execute(&pool)
            .await
            .unwrap();
    };

    insert(f.eng, "eng", "maintainer").await;
    pool.execute(guard)
        .await
        .expect("a maintainer mapping on an ordinary team passes the guard");

    for (team, group, role, named) in [
        (f.eng, "leads", "owner", "sm-okta/leads -> sm-eng (owner)"),
        (
            f.root,
            "everyone",
            "member",
            "sm-okta/everyone -> temper-system (member)",
        ),
        (
            f.gating,
            "staff",
            "watcher",
            "sm-okta/staff -> sm-gate (watcher)",
        ),
    ] {
        insert(team, group, role).await;
        let err = pool
            .execute(guard)
            .await
            .expect_err("the guard refuses a mapping above the ceiling");
        let msg = err.to_string();
        assert!(msg.contains(named), "the guard names {named}, got {msg}");
        assert!(msg.contains("Before this migration"), "{msg}");
        assert!(
            msg.contains("must be deleted") && msg.contains("--confirm-elevated"),
            "the guard says which remedy fits which row: {msg}"
        );
        clear().await;
        insert(f.eng, "eng", "maintainer").await;
    }
}

/// The gating designation is read under the gating lock on the create's transaction: a repoint
/// onto the target team that is in flight makes the create wait, and once it commits the create is
/// refused for want of confirmation, having written nothing.
///
/// FAILS IF the instance-team test reads the gating slug outside `_gating_team_lock`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_gating_repoint_in_flight_onto_the_target_needs_confirmation(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;

    let mut repoint = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *repoint)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'sm-eng' WHERE id = 1")
        .execute(&mut *repoint)
        .await
        .expect("repoint the gating slug onto the target");

    let contender = tokio::spawn({
        let pool = pool.clone();
        let r = req(f.eng, "eng", TeamRole::Member, false);
        async move { saml_mapping_service::create(&pool, &admin, &r).await }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)))",
        )
        .bind(pid)
        .fetch_one(&pool)
        .await
        .unwrap();
        if waiting {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the create never waited on the repoint"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    repoint.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert_needs_confirmation(err, "instance team");
    assert!(mappings(&pool).await.is_empty());
    assert!(records(&pool).await.is_empty());
}

/// A re-run of an identical mapping is a no-op without the confirmation, even for an `owner` or
/// instance-team mapping: the existing row is answered first. The bootstrap script re-runs every
/// mapping it declares.
///
/// FAILS IF the confirmation is asked before the existing row is read.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_identical_rerun_is_a_noop_without_confirmation(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;
    for (team, group, role) in [
        (f.eng, "leads", TeamRole::Owner),
        (f.gating, "staff", TeamRole::Member),
    ] {
        saml_mapping_service::create(&pool, &admin, &req(team, group, role, true))
            .await
            .expect("the confirmed create");
        let again = saml_mapping_service::create(&pool, &admin, &req(team, group, role, false))
            .await
            .expect("an identical re-run needs no confirmation");
        assert!(!again.created && !again.confirmed, "{again:?}");
    }
    assert_eq!(
        records(&pool).await.len(),
        2,
        "the re-runs recorded nothing"
    );
    // A different role on the same key is still a named conflict, confirmation or not.
    let err =
        saml_mapping_service::create(&pool, &admin, &req(f.eng, "leads", TeamRole::Member, false))
            .await
            .unwrap_err();
    assert!(
        matches!(&err, ApiError::Conflict(m) if m.contains("`owner`")),
        "{err:?}"
    );
}

/// The create arm takes the shared gating lock BEFORE the team row. A rename takes the gating lock
/// exclusively and then the team row, so the reverse order would deadlock with it. Pinned by
/// holding the exclusive gating lock: the create waits on it, and while it waits the team row is
/// still free to lock `FOR UPDATE NOWAIT` (it would not be had the create locked the row first).
///
/// FAILS IF the create arm locks the team row before the gating lock.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_create_takes_the_gating_lock_before_the_team_row(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;

    let mut holder = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    sqlx::query("SELECT _gating_team_lock(true)")
        .execute(&mut *holder)
        .await
        .unwrap();

    let contender = tokio::spawn({
        let pool = pool.clone();
        let r = req(f.eng, "eng", TeamRole::Member, false);
        async move { saml_mapping_service::create(&pool, &admin, &r).await }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)))",
        )
        .bind(pid)
        .fetch_one(&pool)
        .await
        .unwrap();
        if waiting {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the create never waited on the gating lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let mut probe = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR UPDATE NOWAIT")
        .bind(f.eng)
        .fetch_one(&mut *probe)
        .await
        .expect("the waiting create holds no lock on the team row");
    probe.rollback().await.unwrap();

    holder.commit().await.unwrap();
    contender
        .await
        .unwrap()
        .expect("the create completes once the gating lock is released");
}
