//! Witnesses for the decided-history cap on re-Request cycling
//! (`access_service::MAX_DECIDED_JOIN_REQUESTS`) — the standing machine's own
//! admission bound beside the self-service Request/Withdraw pair.
//!
//! Authored in-build, per the task's criterion: each must fail against the
//! pre-mechanism state. The cap-bites witness drives the exact cycle the bound exists
//! to stop — with the seam's `None` (default-off) posture, so the bite is the cap's and
//! not the rate limit's.

#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ids::ProfileId;
use uuid::Uuid;

use temper_services::services::access_service;

/// Seed the gating team + settings so a join request can be filed.
async fn seed_gating_team(pool: &sqlx::PgPool) -> Uuid {
    let team_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('cap-gating','History Cap Gating') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("seed gating team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='cap-gating' WHERE id=1")
        .execute(pool)
        .await
        .expect("point gating at the team");
    team_id
}

/// A principal in `denied` standing — the standing from which `Act::Request` is legal.
async fn denied_profile(pool: &sqlx::PgPool, email: &str) -> Uuid {
    let profile = common::fixtures::create_test_profile(pool, email).await;
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'denied') \
         ON CONFLICT (profile_id) DO UPDATE SET state = 'denied', updated = now()",
    )
    .bind(profile)
    .execute(pool)
    .await
    .expect("set standing to denied");
    profile
}

fn request_for(profile: Uuid) -> access_service::CreateJoinRequestParams {
    access_service::CreateJoinRequestParams {
        profile_id: ProfileId::from(profile),
        message: None,
        source: "test".to_owned(),
        accepted_terms_version: None,
    }
}

/// Seed `n` decided join-request rows for this principal — alternating `withdrawn` and
/// `rejected`, the two cycling terminals, so the cap's predicate is exercised across
/// both rather than one.
async fn seed_decided_history(pool: &sqlx::PgPool, team_id: Uuid, profile: Uuid, n: i64) {
    for i in 0..n {
        let status = if i % 2 == 0 { "withdrawn" } else { "rejected" };
        sqlx::query(
            "INSERT INTO kb_join_requests \
                 (id, team_id, requesting_profile_id, status, source, created, updated) \
             VALUES ($1, $2, $3, $4::join_request_status, 'test', now(), now())",
        )
        .bind(Uuid::now_v7())
        .bind(team_id)
        .bind(profile)
        .bind(status)
        .execute(pool)
        .await
        .expect("seed decided row");
    }
}

/// The refusal the cap owes: count on the table, and the principal exactly where they
/// were. Shared by the witnesses that refuse.
async fn assert_refusal_wrote_nothing(pool: &sqlx::PgPool, profile: Uuid, expected_decided: i64) {
    let (decided, pending): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE status <> 'pending'), \
                count(*) FILTER (WHERE status = 'pending') \
           FROM kb_join_requests WHERE requesting_profile_id = $1",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("count rows");
    assert_eq!(
        decided, expected_decided,
        "the refusal must not reap decided history"
    );
    assert_eq!(pending, 0, "the refused request must not have filed a row");

    let standing: String =
        sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id = $1")
            .bind(profile)
            .fetch_one(pool)
            .await
            .expect("standing row");
    assert_eq!(
        standing, "denied",
        "the refused request must not have moved standing"
    );
}

/// **Cap-bites witness.** A principal at the cap is refused a further request —
/// with the seam absent (`None`, the shipped default-off posture), so the bite is the
/// cap's own — riding the standing machine's refusal vocabulary (the interim
/// `BadRequest` an illegal transition carries), never the seam's 429 — and reaping
/// nothing. Fails while no cap exists: the request files happily as row cap+1 and
/// standing moves to `requested`.
// FAILS IF: the cap check is removed from `create_join_request`, or the refusal is ever
// re-shaped onto the seam's error surface.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_principal_at_the_decided_cap_is_refused(pool: sqlx::PgPool) {
    let team_id = seed_gating_team(&pool).await;
    let profile = denied_profile(&pool, "cap-bites@test.example.com").await;
    seed_decided_history(
        &pool,
        team_id,
        profile,
        access_service::MAX_DECIDED_JOIN_REQUESTS,
    )
    .await;

    let err = access_service::create_join_request(&pool, request_for(profile), None)
        .await
        .expect_err("a principal at the cap must be refused");
    let temper_services::error::ApiError::BadRequest(reason) = &err else {
        panic!(
            "the cap refusal rides the standing machine's vocabulary, not the seam's: got {err:?}"
        )
    };
    assert!(
        reason.contains("decided access requests"),
        "the refusal names the fact that refuses: {reason}"
    );

    assert_refusal_wrote_nothing(&pool, profile, access_service::MAX_DECIDED_JOIN_REQUESTS).await;
}

/// **Boundary witness.** One row short of the cap the request files; completing the
/// cycle through the machine's own lifecycle lands the principal exactly at the cap,
/// and the next request refuses. Fails if the cap is off by one in either direction.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_boundary_is_the_cap_itself(pool: sqlx::PgPool) {
    let team_id = seed_gating_team(&pool).await;
    let profile = denied_profile(&pool, "cap-edge@test.example.com").await;
    seed_decided_history(
        &pool,
        team_id,
        profile,
        access_service::MAX_DECIDED_JOIN_REQUESTS - 1,
    )
    .await;

    // Row cap-1: the pump still turns for a genuine applicant.
    access_service::create_join_request(&pool, request_for(profile), None)
        .await
        .expect("a principal under the cap files");
    access_service::withdraw_request(&pool, ProfileId::from(profile))
        .await
        .expect("withdraw");

    // Now at the cap via the machine's own lifecycle, not seeding: the next refuses.
    assert!(
        matches!(
            access_service::create_join_request(&pool, request_for(profile), None).await,
            Err(temper_services::error::ApiError::BadRequest(_))
        ),
        "the request after the cycle that reached the cap must refuse"
    );
    assert_refusal_wrote_nothing(&pool, profile, access_service::MAX_DECIDED_JOIN_REQUESTS).await;
}

/// **Back-out witness.** The cap binds the Request act only: a principal at the cap
/// holding a pending row can still withdraw it. The bound refuses further filings; it
/// never strands a principal in a pending state they cannot leave.
// FAILS IF: the guard is ever extended to `withdraw_request` (mutation-proven: the
// back-out goes red when the cap rides Withdraw too).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_cap_never_strands_a_pending_request(pool: sqlx::PgPool) {
    let team_id = seed_gating_team(&pool).await;
    let profile = denied_profile(&pool, "cap-backout@test.example.com").await;
    seed_decided_history(
        &pool,
        team_id,
        profile,
        access_service::MAX_DECIDED_JOIN_REQUESTS,
    )
    .await;

    // The shape the cap can meet in the wild: a pending row filed before the cap
    // landed, standing at `requested` to match.
    sqlx::query(
        "INSERT INTO kb_join_requests \
             (id, team_id, requesting_profile_id, status, source, created, updated) \
         VALUES ($1, $2, $3, 'pending', 'test', now(), now())",
    )
    .bind(Uuid::now_v7())
    .bind(team_id)
    .bind(profile)
    .execute(&pool)
    .await
    .expect("seed pending row");
    sqlx::query("UPDATE kb_principal_standing SET state = 'requested', updated = now() WHERE profile_id = $1")
        .bind(profile)
        .execute(&pool)
        .await
        .expect("set standing to requested");

    access_service::withdraw_request(&pool, ProfileId::from(profile))
        .await
        .expect("withdrawal stays available at the cap");
}
