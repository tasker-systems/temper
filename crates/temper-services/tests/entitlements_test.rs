#![cfg(feature = "test-db")]
//! `get_entitlements` — the authoritative answer to "may I use this instance?".
//!
//! The defect these pin: `temper auth status` used to answer from the join-request *queue*, whose
//! empty result it read as denial. A principal admitted by any path other than the queue — the
//! instance owner included — has no row there, so the queue reports denial for exactly the
//! population most likely to hold access. Standing is the authority; the queue is a side record.
//!
//! **No disclosure narrowing is tested here because none exists.** A three-variant reportable
//! standing was built and reverted: see `temper_core::types::access_gate::Entitlements` for why
//! withholding revocation is not this system's posture (spec D15 gives a revoked principal an
//! appeal, which requires knowing they were revoked).

use sqlx::PgPool;
use temper_core::types::access_gate::JoinRequestStatus;
use temper_core::types::ids::ProfileId;
use temper_principal::Standing;
use temper_services::services::access_service;

async fn a_profile(pool: &PgPool, handle: &str) -> uuid::Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Move a principal to `state` through the SQL committer.
///
/// **This is the committer underneath the admin door, not the door itself.** `standing_service::apply`
/// decides legality and demotes governance on Revoke/Deactivate; this function does neither (the
/// migration says so in capitals). That is fine for arranging a state cheaply, and it is why no test
/// here asserts anything about `is_admin` — the fixture cannot maintain the admin/standing
/// invariant, so a test of it would be measuring the fixture.
async fn set_standing(pool: &PgPool, profile: uuid::Uuid, act: &str, state: &str) {
    let committed: String =
        sqlx::query_scalar("SELECT principal_standing_apply($1,$2,$3,NULL,NULL)")
            .bind(profile)
            .bind(act)
            .bind(state)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(committed, state, "committer echoed a different state");
}

async fn a_gating_team(pool: &PgPool) -> uuid::Uuid {
    let team: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(pool)
        .await
        .unwrap();
    team
}

async fn a_join_request(pool: &PgPool, team: uuid::Uuid, profile: uuid::Uuid, status: &str) {
    sqlx::query(
        "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, status, source) \
         VALUES (gen_random_uuid(), $1, $2, $3::join_request_status, 'test')",
    )
    .bind(team)
    .bind(profile)
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
}

/// The defect this work was opened for: approved standing, nothing in the queue.
///
/// This is the instance owner's own shape — admitted without ever filing a request — so the bug it
/// pins is not hypothetical. `system_access` must come from standing, never from the queue's
/// emptiness.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn approved_standing_with_no_join_request_still_has_access(pool: PgPool) {
    let p = a_profile(&pool, "approved-never-asked").await;
    set_standing(&pool, p, "provision", "denied").await;
    set_standing(&pool, p, "approve", "approved").await;

    // The precondition the old implementation misread: nothing in the queue.
    let queued: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_join_requests WHERE requesting_profile_id=$1")
            .bind(p)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(queued, 0, "fixture must leave the queue empty");

    let e = access_service::get_entitlements(&pool, ProfileId::from(p))
        .await
        .unwrap();

    assert!(
        e.system_access,
        "standing is the authority; an empty queue is not a denial"
    );
    assert_eq!(e.standing, Some(Standing::Approved));
    assert_eq!(e.join_request_status, None);
}

/// Absence denies — no standing row at all means no access, not an error.
/// This is what makes connection profiles safe by construction (spec D7).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_principal_with_no_standing_row_has_no_access(pool: PgPool) {
    let p = a_profile(&pool, "no-row").await;

    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_principal_standing WHERE profile_id=$1")
            .bind(p)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 0, "fixture must leave the principal without a row");

    let e = access_service::get_entitlements(&pool, ProfileId::from(p))
        .await
        .unwrap();

    assert!(!e.system_access);
    assert_eq!(
        e.standing,
        Some(Standing::Denied),
        "absence denies, and is reported as `denied` — `None` is reserved for an older server"
    );
    assert_eq!(e.join_request_status, None);
}

/// A rejected principal is told they were rejected, and keeps the decision on record.
///
/// This is the case a suppression attempt broke: rejection returns standing to `denied`, so
/// narrowing on standing hid the decline from the person it concerns — while the web surface
/// renders it to them with the reviewer's note. The queue status must survive the standing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_rejected_principal_is_told_they_were_rejected(pool: PgPool) {
    let p = a_profile(&pool, "rejected").await;
    let team = a_gating_team(&pool).await;
    set_standing(&pool, p, "provision", "denied").await;
    a_join_request(&pool, team, p, "rejected").await;

    let e = access_service::get_entitlements(&pool, ProfileId::from(p))
        .await
        .unwrap();

    assert!(!e.system_access);
    assert_eq!(e.standing, Some(Standing::Denied));
    assert_eq!(
        e.join_request_status,
        Some(JoinRequestStatus::Rejected),
        "a decided request must stay visible to the principal it decided"
    );
}

/// A pending request is visible to the asker while they wait.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_pending_request_is_visible_to_the_asker(pool: PgPool) {
    let p = a_profile(&pool, "asked").await;
    let team = a_gating_team(&pool).await;
    set_standing(&pool, p, "provision", "denied").await;
    set_standing(&pool, p, "request", "requested").await;
    a_join_request(&pool, team, p, "pending").await;

    let e = access_service::get_entitlements(&pool, ProfileId::from(p))
        .await
        .unwrap();

    assert!(!e.system_access);
    assert_eq!(e.standing, Some(Standing::Requested));
    assert_eq!(e.join_request_status, Some(JoinRequestStatus::Pending));
}

/// A revoked principal loses access, and the surviving `approved` queue row is reported as it
/// stands.
///
/// Both facts are deliberate. Standing is what governs access, so `system_access` goes false while
/// nothing moves the queue row — the two updates that could are guarded `WHERE status = 'pending'`
/// and the standing transitions never touch that table. Reporting it is consistent with the rest of
/// the system, which tells a revoked principal so on purpose because D15 gives them an appeal.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_revoked_principal_loses_access_and_keeps_the_record(pool: PgPool) {
    let p = a_profile(&pool, "revoked").await;
    let team = a_gating_team(&pool).await;
    set_standing(&pool, p, "provision", "denied").await;
    set_standing(&pool, p, "approve", "approved").await;
    a_join_request(&pool, team, p, "approved").await;
    set_standing(&pool, p, "revoke", "revoked").await;

    let surviving: String = sqlx::query_scalar(
        "SELECT status::text FROM kb_join_requests WHERE requesting_profile_id=$1",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        surviving, "approved",
        "revocation leaves the approved row behind — nothing can move it"
    );

    let e = access_service::get_entitlements(&pool, ProfileId::from(p))
        .await
        .unwrap();

    assert!(!e.system_access, "revocation removes access");
    assert_eq!(
        e.standing,
        Some(Standing::Revoked),
        "revoked is reported as revoked, not folded into denied — D15 gives it a distinct remedy"
    );
    assert_eq!(e.join_request_status, Some(JoinRequestStatus::Approved));
}
