#![cfg(feature = "test-db")]
//! An admin's decision on a join request moves standing only by a legal transition.
//!
//! A pending join request can outlive the standing it was filed under: the person is revoked,
//! deactivated, or approved directly while the request sits in the queue. Deciding that stale row
//! used to write `approved` / `denied` raw, whatever the person's standing had become — so
//! rejecting it laundered Revoked into Denied (D15), and either decision lifted a deactivation that
//! only `Reactivate` may lift. The decision now goes through the transition table on the
//! requester's locked standing, and an illegal one refuses the WHOLE decision: a `409` naming the
//! current standing, the request still pending, and no standing row, log entry or event written.
//!
//! Each stale case below seeds the row the way it was stranded (the standing moved and the request
//! did not), then asserts the refusal and the absence of every write. The live control is the
//! ordinary path: a still-`Requested` person whose request is approved and rejected.

use sqlx::PgPool;
use temper_core::types::access_gate::JoinRequestStatus;
use temper_core::types::ids::ProfileId;
use temper_principal::{Provisioner, Standing};
use temper_services::auth::SystemAdmin;
use temper_services::error::ApiError;
use temper_services::services::{access_service, standing_service};
use temper_services::test_support;
use uuid::Uuid;

async fn a_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn an_admin(pool: &PgPool) -> (Uuid, SystemAdmin) {
    let id = a_profile(pool, "the-admin").await;
    test_support::approved_admin(pool, id).await;
    // The emitter an approval's recorded membership write attributes to (`<handle>@web`).
    sqlx::query("INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, 'the-admin@web', '{}'::jsonb)")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    let proof = test_support::system_admin_proof_for(pool, id).await;
    (id, proof)
}

/// A gating team the requests are filed against.
async fn a_gating_team(pool: &PgPool) {
    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ('gate','Gate') ON CONFLICT DO NOTHING")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'gate' WHERE id = 1")
        .execute(pool)
        .await
        .unwrap();
}

/// A person born `Denied` who files a join request through the real door: `Requested`, with a
/// pending row. Returns the person and the request.
async fn a_requester(pool: &PgPool, handle: &str) -> (ProfileId, Uuid) {
    let id = a_profile(pool, handle).await;
    let person = ProfileId::from(id);
    standing_service::provision(pool, person, Provisioner::OauthFirstLogin)
        .await
        .expect("born denied");
    let row = access_service::create_join_request(
        pool,
        &test_support::human_principal_for(pool, id).await,
        access_service::CreateJoinRequestParams {
            message: None,
            source: "test".to_string(),
            accepted_terms_version: None,
        },
        None,
    )
    .await
    .expect("file a join request");
    assert_eq!(standing(pool, person).await, Some(Standing::Requested));
    (person, row.id)
}

/// Move standing the way the pre-fix code could, leaving the request row untouched — the stranding
/// this file is about. Raw on purpose: the doors that strand a row no longer exist.
async fn strand(pool: &PgPool, person: ProfileId, act: &str, to: &str, admin: Uuid) {
    sqlx::query("SELECT principal_standing_apply($1,$2,$3,$4,NULL)")
        .bind(*person)
        .bind(act)
        .bind(to)
        .bind(admin)
        .execute(pool)
        .await
        .expect("strand");
}

async fn standing(pool: &PgPool, person: ProfileId) -> Option<Standing> {
    standing_service::load(pool, person).await.unwrap()
}

async fn request_status(pool: &PgPool, request: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM kb_join_requests WHERE id = $1")
        .bind(request)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Everything a decision could write about the person: their standing log and the events.
async fn writes(pool: &PgPool, person: ProfileId) -> (i64, i64) {
    let log: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_principal_standing_events WHERE profile_id = $1",
    )
    .bind(*person)
    .fetch_one(pool)
    .await
    .unwrap();
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .unwrap();
    (log, events)
}

async fn decide(
    pool: &PgPool,
    admin: &SystemAdmin,
    request: Uuid,
    decision: JoinRequestStatus,
) -> Result<(), ApiError> {
    access_service::review_request(
        pool,
        admin,
        access_service::ReviewRequestParams {
            request_id: request,
            decision,
            decision_note: Some("stale queue".to_string()),
        },
    )
    .await
    .map(|_| ())
}

/// Assert the decision is refused with a 409 naming `now`, and that nothing moved.
async fn assert_refused_and_unwritten(
    pool: &PgPool,
    admin: &SystemAdmin,
    person: ProfileId,
    request: Uuid,
    decision: JoinRequestStatus,
    now: Standing,
    names: &str,
) {
    let before = writes(pool, person).await;
    let err = decide(pool, admin, request, decision)
        .await
        .expect_err("a stale decision must be refused");
    match &err {
        ApiError::Conflict(msg) => {
            assert!(msg.contains(names), "the 409 names what was hit: {msg}");
            assert!(msg.contains("Nothing was written"), "{msg}");
        }
        other => panic!("expected a 409 Conflict, got {other:?}"),
    }
    assert_eq!(
        standing(pool, person).await,
        Some(now),
        "standing did not move"
    );
    assert_eq!(
        request_status(pool, request).await,
        "pending",
        "the request stays pending"
    );
    assert_eq!(
        writes(pool, person).await,
        before,
        "no standing log entry and no event"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn rejecting_a_revoked_persons_stale_request_does_not_launder_them_into_denied(pool: PgPool) {
    a_gating_team(&pool).await;
    let (admin_id, admin) = an_admin(&pool).await;
    let (person, request) = a_requester(&pool, "revoked-one").await;
    strand(&pool, person, "approve", "approved", admin_id).await;
    access_service::admin_revoke(&pool, &admin, person, "left".to_string())
        .await
        .expect("revoke");

    assert_refused_and_unwritten(
        &pool,
        &admin,
        person,
        request,
        JoinRequestStatus::Rejected,
        Standing::Revoked,
        "revoked",
    )
    .await;
}

/// Approve from Revoked is legal in the table (readmission, D16), but not by way of an old queue
/// row: the join-request door refuses it and names the deliberate door.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn approving_a_revoked_persons_stale_request_does_not_readmit_them(pool: PgPool) {
    a_gating_team(&pool).await;
    let (admin_id, admin) = an_admin(&pool).await;
    let (person, request) = a_requester(&pool, "revoked-two").await;
    strand(&pool, person, "approve", "approved", admin_id).await;
    access_service::admin_revoke(&pool, &admin, person, "left".to_string())
        .await
        .expect("revoke");

    assert_refused_and_unwritten(
        &pool,
        &admin,
        person,
        request,
        JoinRequestStatus::Approved,
        Standing::Revoked,
        "temper admin access approve",
    )
    .await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn approving_a_deactivated_persons_request_does_not_lift_the_deactivation(pool: PgPool) {
    a_gating_team(&pool).await;
    let (_, admin) = an_admin(&pool).await;
    let (person, request) = a_requester(&pool, "deactivated-approve").await;
    // The real door: deactivation from `Requested` parks the request rather than closing it.
    access_service::admin_deactivate(&pool, &admin, person)
        .await
        .expect("deactivate");
    assert_eq!(request_status(&pool, request).await, "pending");

    assert_refused_and_unwritten(
        &pool,
        &admin,
        person,
        request,
        JoinRequestStatus::Approved,
        Standing::Deactivated,
        "reactivate",
    )
    .await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn rejecting_a_deactivated_persons_request_does_not_lift_the_deactivation(pool: PgPool) {
    a_gating_team(&pool).await;
    let (_, admin) = an_admin(&pool).await;
    let (person, request) = a_requester(&pool, "deactivated-reject").await;
    access_service::admin_deactivate(&pool, &admin, person)
        .await
        .expect("deactivate");

    assert_refused_and_unwritten(
        &pool,
        &admin,
        person,
        request,
        JoinRequestStatus::Rejected,
        Standing::Deactivated,
        "reactivate",
    )
    .await;

    // The parked request is decidable again once the person is reactivated to `Requested`.
    access_service::admin_reactivate(&pool, &admin, person)
        .await
        .expect("reactivate");
    assert_eq!(standing(&pool, person).await, Some(Standing::Requested));
    decide(&pool, &admin, request, JoinRequestStatus::Rejected)
        .await
        .expect("a reactivated person's request is decidable");
    assert_eq!(standing(&pool, person).await, Some(Standing::Denied));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn approving_an_already_approved_persons_request_writes_no_noop(pool: PgPool) {
    a_gating_team(&pool).await;
    let (admin_id, admin) = an_admin(&pool).await;
    let (person, request) = a_requester(&pool, "approved-already").await;
    strand(&pool, person, "approve", "approved", admin_id).await;

    assert_refused_and_unwritten(
        &pool,
        &admin,
        person,
        request,
        JoinRequestStatus::Approved,
        Standing::Approved,
        "already approved",
    )
    .await;
}

/// The live control: a still-`Requested` person's request is decided exactly as before.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_live_request_is_approved_and_rejected_as_before(pool: PgPool) {
    a_gating_team(&pool).await;
    let (_, admin) = an_admin(&pool).await;

    let (approved, approved_req) = a_requester(&pool, "live-approve").await;
    decide(&pool, &admin, approved_req, JoinRequestStatus::Approved)
        .await
        .expect("approve a live request");
    assert_eq!(standing(&pool, approved).await, Some(Standing::Approved));
    assert_eq!(request_status(&pool, approved_req).await, "approved");

    let (rejected, rejected_req) = a_requester(&pool, "live-reject").await;
    decide(&pool, &admin, rejected_req, JoinRequestStatus::Rejected)
        .await
        .expect("reject a live request");
    assert_eq!(standing(&pool, rejected).await, Some(Standing::Denied));
    assert_eq!(request_status(&pool, rejected_req).await, "rejected");

    // The decision note still rides the standing log, as the raw write used to carry it.
    let reason: Option<String> = sqlx::query_scalar(
        "SELECT reason FROM kb_principal_standing_events \
          WHERE profile_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(*rejected)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reason.as_deref(), Some("stale queue"));
}

/// The stale-row source is closed: a direct approval answers the open request in the same
/// transaction, so a later revoke has no pending row to strand.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_direct_approval_closes_the_open_join_request(pool: PgPool) {
    a_gating_team(&pool).await;
    let (admin_id, admin) = an_admin(&pool).await;
    let (person, request) = a_requester(&pool, "direct-approve").await;

    access_service::admin_approve(&pool, &admin, person)
        .await
        .expect("direct approve");
    assert_eq!(request_status(&pool, request).await, "approved");
    let reviewer: Option<Uuid> =
        sqlx::query_scalar("SELECT reviewed_by_profile_id FROM kb_join_requests WHERE id = $1")
            .bind(request)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(reviewer, Some(admin_id));

    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_join_requests \
          WHERE requesting_profile_id = $1 AND status = 'pending'",
    )
    .bind(*person)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 0);
}

/// Promotion does not lift a deactivation: refused, naming Reactivate, with no grant written.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn promoting_a_deactivated_person_is_refused_and_names_reactivate(pool: PgPool) {
    a_gating_team(&pool).await;
    let (_, admin) = an_admin(&pool).await;
    let (person, _) = a_requester(&pool, "deactivated-promote").await;
    access_service::admin_deactivate(&pool, &admin, person)
        .await
        .expect("deactivate");
    let before = writes(&pool, person).await;

    let err = access_service::promote_admin(&pool, &admin, *person, None)
        .await
        .expect_err("promotion from deactivated must be refused");
    match &err {
        ApiError::Conflict(msg) => assert!(msg.contains("reactivate"), "{msg}"),
        other => panic!("expected a 409 Conflict, got {other:?}"),
    }
    assert_eq!(standing(&pool, person).await, Some(Standing::Deactivated));
    assert_eq!(writes(&pool, person).await, before);
    let governs: bool = sqlx::query_scalar("SELECT is_system_admin($1)")
        .bind(*person)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!governs, "no governance grant was written");
    let owner_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members WHERE profile_id = $1 AND role = 'owner' \
            AND team_id = (SELECT id FROM kb_teams WHERE slug = 'gate')",
    )
    .bind(*person)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(owner_rows, 0, "no team owner row was written");
}

/// A promotion is an admission decision too: promoting a `Requested` person approves them and
/// answers their open join request in the same transaction.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn promoting_a_requested_person_closes_their_join_request(pool: PgPool) {
    a_gating_team(&pool).await;
    let (_, admin) = an_admin(&pool).await;
    let (person, request) = a_requester(&pool, "requested-promote").await;

    access_service::promote_admin(&pool, &admin, *person, None)
        .await
        .expect("promote a requested person");
    assert_eq!(standing(&pool, person).await, Some(Standing::Approved));
    assert_eq!(request_status(&pool, request).await, "approved");
    let reason: Option<String> = sqlx::query_scalar(
        "SELECT reason FROM kb_principal_standing_events \
          WHERE profile_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(*person)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reason.as_deref(), Some("system admin promotion"));
}
