#![cfg(feature = "test-db")]
//! `kb_principal_review_requests` — the inbox signal that had no inbox.
//!
//! The table (migration `20260721000020_review_requests.sql`) calls itself
//! `'AN INBOX SIGNAL ONLY'`. Until now it had exactly one code path in the whole workspace: the
//! `INSERT` in `create_review_request`. Nothing selected from it, and nothing ever wrote
//! `decided_at` / `decided_by` / `decision_note` — three columns that existed and were unreachable.
//!
//! That is not merely a missing view. `idx_principal_review_one_open` is
//! `UNIQUE (profile_id) WHERE decided_at IS NULL`, so a guard that can never be released is a
//! **permanent** lockout: a principal who files one review can never file another, because the only
//! thing that would clear the guard did not exist. `closing_a_review_releases_the_one_open_guard`
//! is the witness for that, and it is the reason this file exists.
//!
//! What closing a review must NOT do is the other half. D15's whole point is that reconsideration
//! cannot launder a revocation — the marker moves nothing, and the admin's actual answer is a
//! separate `Approve`. `closing_a_review_moves_no_standing` pins that, because "close" is exactly
//! the verb someone would later be tempted to make grant access.

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_principal::Standing;
use temper_services::auth::SystemAdmin;
use temper_services::error::ApiError;
use temper_services::services::{access_service, standing_service};
use temper_services::test_support;

async fn a_profile(pool: &PgPool, handle: &str) -> uuid::Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// An admin who can act on the inbox.
async fn an_admin(pool: &PgPool, handle: &str) -> (uuid::Uuid, SystemAdmin) {
    let id = a_profile(pool, handle).await;
    test_support::approved_admin(pool, id).await;
    let proof = test_support::system_admin_proof_for(pool, id).await;
    (id, proof)
}

/// A principal in `Revoked` — the only standing from which `RequestReview` is legal (D15).
async fn a_revoked_principal(pool: &PgPool, admin: &SystemAdmin, handle: &str) -> ProfileId {
    let id = a_profile(pool, handle).await;
    test_support::approve(pool, id).await;
    let subject = ProfileId::from(id);
    access_service::admin_revoke(pool, admin, subject, "spec violation".to_string())
        .await
        .expect("revoke from approved is legal");
    subject
}

async fn file_a_review(pool: &PgPool, subject: ProfileId, message: &str) -> Result<(), ApiError> {
    access_service::create_review_request(
        pool,
        access_service::CreateReviewRequestParams {
            profile_id: subject,
            message: Some(message.to_string()),
        },
    )
    .await
}

async fn standing_of(pool: &PgPool, subject: ProfileId) -> Option<Standing> {
    standing_service::load(pool, subject).await.unwrap()
}

/// An open review reaches the admin queue, carrying the message and the requester's identity.
///
/// The identity is the point: an admin deciding a reconsideration needs to know *who*, and the row
/// on its own is a bare `profile_id`. This is the same join `list_pending_requests` does for join
/// requests, for the same reason.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_open_review_reaches_the_admin_inbox(pool: PgPool) {
    let (_, admin) = an_admin(&pool, "inbox-admin").await;
    let subject = a_revoked_principal(&pool, &admin, "penitent").await;

    file_a_review(
        &pool,
        subject,
        "I have read the policy and would like to return",
    )
    .await
    .expect("a revoked principal may ask for reconsideration");

    let open = access_service::list_open_review_requests(&pool, &admin)
        .await
        .expect("an admin may read the inbox");

    assert_eq!(open.len(), 1, "the filed review is visible: {open:?}");
    assert_eq!(open[0].profile_id, *subject);
    assert_eq!(open[0].handle, "penitent", "the requester is identified");
    assert_eq!(
        open[0].message.as_deref(),
        Some("I have read the policy and would like to return"),
    );
}

/// **The lockout.** Closing a review releases `idx_principal_review_one_open`, so a principal
/// revoked a second time can ask a second time.
///
/// Before this change there was no way to set `decided_at`, so the partial unique index never
/// released and the second `RequestReview` died on a `23505` — permanently, for the life of the
/// profile. The sequence below is entirely reachable with the commands that already shipped:
/// `temper auth request-review`, `temper admin access approve`, `temper admin access revoke`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn closing_a_review_releases_the_one_open_guard(pool: PgPool) {
    let (_, admin) = an_admin(&pool, "guard-admin").await;
    let subject = a_revoked_principal(&pool, &admin, "twice-revoked").await;

    file_a_review(&pool, subject, "first ask").await.unwrap();

    let open = access_service::list_open_review_requests(&pool, &admin)
        .await
        .unwrap();
    access_service::close_review_request(
        &pool,
        &admin,
        access_service::CloseReviewRequestParams {
            request_id: open[0].id,
            decision_note: Some("reinstated after conversation".to_string()),
        },
    )
    .await
    .expect("an admin may close an open review");

    // Reinstated, then revoked again — the real path back to `Revoked`.
    access_service::admin_approve(&pool, &admin, subject)
        .await
        .unwrap();
    access_service::admin_revoke(&pool, &admin, subject, "relapse".to_string())
        .await
        .unwrap();

    file_a_review(&pool, subject, "second ask")
        .await
        .expect("with the first review closed, a second ask is not a duplicate");

    let open = access_service::list_open_review_requests(&pool, &admin)
        .await
        .unwrap();
    assert_eq!(open.len(), 1, "exactly the new ask is open: {open:?}");
    assert_eq!(open[0].message.as_deref(), Some("second ask"));
}

/// The guard still does its job while a review *is* open (D15 obligation 2). Closing must release
/// it; nothing here may loosen it.
///
/// Asserted as a `Conflict` rather than "an error", because the `23505` → `Conflict` mapping is what
/// makes this a refusal the caller can read instead of a 500.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_open_guard_still_refuses_a_second_review(pool: PgPool) {
    let (_, admin) = an_admin(&pool, "dup-admin").await;
    let subject = a_revoked_principal(&pool, &admin, "insistent").await;

    file_a_review(&pool, subject, "first").await.unwrap();
    let err = file_a_review(&pool, subject, "second")
        .await
        .expect_err("a second open review is a duplicate");

    match err {
        // The message is the assertion, not just the variant. The generic `23505` mapping renders
        // "Resource already exists", which tells a locked-out principal nothing about what exists
        // or what to do — and this refusal is one of the few things they can still reach.
        ApiError::Conflict(msg) => assert_eq!(msg, access_service::REVIEW_ALREADY_OPEN),
        other => panic!("the duplicate guard must refuse readably, got {other:?}"),
    }
}

/// **Closing a review moves no standing** (D15). The marker is an inbox signal; the admin's actual
/// answer is a separate `Approve`.
///
/// This is the tempting change — "close" sounds like "resolve", and resolving a reconsideration
/// sounds like granting it. If closing ever admitted anyone, a revocation could be laundered by the
/// revoked principal's own request plus one careless click, which is precisely what the migration's
/// `COMMENT ON TABLE` warns against.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn closing_a_review_moves_no_standing(pool: PgPool) {
    let (_, admin) = an_admin(&pool, "standing-admin").await;
    let subject = a_revoked_principal(&pool, &admin, "still-out").await;
    file_a_review(&pool, subject, "please").await.unwrap();

    let open = access_service::list_open_review_requests(&pool, &admin)
        .await
        .unwrap();
    access_service::close_review_request(
        &pool,
        &admin,
        access_service::CloseReviewRequestParams {
            request_id: open[0].id,
            decision_note: Some("declined".to_string()),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        standing_of(&pool, subject).await,
        Some(Standing::Revoked),
        "closing the signal must leave the revocation exactly where it was"
    );
}

/// A closed review leaves the inbox — the queue is what is *outstanding*, not a history.
///
/// Without this the count warmup reports would only ever grow, which is the failure mode that
/// teaches a reader to ignore a count.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_closed_review_leaves_the_inbox(pool: PgPool) {
    let (admin_id, admin) = an_admin(&pool, "closing-admin").await;
    let subject = a_revoked_principal(&pool, &admin, "departing").await;
    file_a_review(&pool, subject, "please").await.unwrap();

    let open = access_service::list_open_review_requests(&pool, &admin)
        .await
        .unwrap();
    access_service::close_review_request(
        &pool,
        &admin,
        access_service::CloseReviewRequestParams {
            request_id: open[0].id,
            decision_note: None,
        },
    )
    .await
    .unwrap();

    assert!(
        access_service::list_open_review_requests(&pool, &admin)
            .await
            .unwrap()
            .is_empty(),
        "a decided review is no longer outstanding"
    );

    // The decision is still recorded — leaving the queue is not the same as being forgotten.
    let (decided_at, decided_by): (Option<chrono::DateTime<chrono::Utc>>, Option<uuid::Uuid>) =
        sqlx::query_as(
            "SELECT decided_at, decided_by FROM kb_principal_review_requests WHERE id=$1",
        )
        .bind(open[0].id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(decided_at.is_some(), "the decision is stamped");
    assert_eq!(
        decided_by,
        Some(admin_id),
        "and attributed to the admin who made it"
    );
}

/// Closing a review that is already closed is a `NotFound`, not a silent success.
///
/// Two admins working the same queue is the ordinary case, and the second one deserves to be told
/// their click did nothing rather than believe they decided it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn closing_an_already_closed_review_refuses(pool: PgPool) {
    let (_, admin) = an_admin(&pool, "race-admin").await;
    let subject = a_revoked_principal(&pool, &admin, "raced").await;
    file_a_review(&pool, subject, "please").await.unwrap();

    let open = access_service::list_open_review_requests(&pool, &admin)
        .await
        .unwrap();
    let params = || access_service::CloseReviewRequestParams {
        request_id: open[0].id,
        decision_note: None,
    };

    access_service::close_review_request(&pool, &admin, params())
        .await
        .unwrap();
    let err = access_service::close_review_request(&pool, &admin, params())
        .await
        .expect_err("the second close finds nothing open");

    assert!(
        matches!(err, ApiError::NotFound(_)),
        "an already-decided review is not there to decide, got {err:?}"
    );
}

/// **The sequence the feature claims to fix — with no explicit close in it.**
///
/// Closing by hand releases the guard, which `closing_a_review_releases_the_one_open_guard` proves.
/// But the operator path that actually happens is readmission: an admin approves the principal and
/// never opens the reconsideration queue at all. If approval does not answer the request, the row
/// stays open forever, the guard never releases on the likeliest path, and the stale row inflates
/// the admin queue and warmup's count indefinitely — while `REVIEW_ALREADY_OPEN` tells the locked-out
/// principal "an admin has not decided it yet", which by then is false.
///
/// **The direction is what makes this safe under D15.** The review must never be an admission
/// INPUT — that is the conjunction D2 forbids, and nothing here reads the review to decide
/// anything. This is the opposite arrow: the decision was already made, on the standing log where
/// admission decisions belong, and the marker is being kept consistent with it afterwards.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn readmission_answers_the_open_review(pool: PgPool) {
    let (admin_id, admin) = an_admin(&pool, "readmitting-admin").await;
    let subject = a_revoked_principal(&pool, &admin, "returning").await;
    file_a_review(&pool, subject, "first ask").await.unwrap();

    // The admin readmits WITHOUT ever visiting `temper admin reviews`.
    access_service::admin_approve(&pool, &admin, subject)
        .await
        .expect("approve from revoked is legal");

    assert!(
        access_service::list_open_review_requests(&pool, &admin)
            .await
            .unwrap()
            .is_empty(),
        "readmission answered the request, so it leaves the queue instead of inflating it forever"
    );

    let decided_by: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT decided_by FROM kb_principal_review_requests WHERE profile_id=$1",
    )
    .bind(*subject)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        decided_by,
        Some(admin_id),
        "attributed to the admin whose approval answered it"
    );

    // And the guard released, so a second revocation can be appealed at all.
    access_service::admin_revoke(&pool, &admin, subject, "relapse".to_string())
        .await
        .unwrap();
    file_a_review(&pool, subject, "second ask")
        .await
        .expect("the guard releases on the operator path, not only the manual one");
}
