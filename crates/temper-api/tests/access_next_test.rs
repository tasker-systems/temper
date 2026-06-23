#![cfg(all(feature = "test-db", feature = "next-backend"))]
//! Dark-launch (`next-backend`) access path: the `*_next` join-request lifecycle
//! variants write the operational tables in the `temper_next` substrate.
//!
//! Admin/operational events are firewalled from the cognition ledger
//! (`kb_events`): these variants emit NO event. The audit trail lives on
//! `kb_join_requests` (status / reviewed_by_profile_id / timestamps) plus the
//! `kb_team_members` membership row created on approval.

use sqlx::PgPool;
use uuid::Uuid;

use temper_api::services::access_service::{self, CreateJoinRequestParams, ReviewRequestParams};
use temper_core::types::access_gate::JoinRequestStatus;

/// Configure the singleton system settings into `invite_only` with a gating
/// team slug. `get_system_settings` reads UNQUALIFIED `kb_system_settings`
/// (resolves to `public` under the test's default search_path), so the settings
/// row is seeded there; the gating team + profiles live in `temper_next`.
async fn arrange_invite_only(pool: &PgPool, gating_slug: &str) -> Uuid {
    sqlx::query(
        r#"INSERT INTO public.kb_system_settings (id, access_mode, gating_team_slug, updated)
           VALUES (1, 'invite_only', $1, now())
           ON CONFLICT (id) DO UPDATE
             SET access_mode = 'invite_only', gating_team_slug = $1, updated = now()"#,
    )
    .bind(gating_slug)
    .execute(pool)
    .await
    .expect("configure invite_only settings");

    let team_id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO temper_next.kb_teams (id, slug, name)
           VALUES ($1, $2, 'Gating Team')"#,
    )
    .bind(team_id)
    .bind(gating_slug)
    .execute(pool)
    .await
    .expect("seed gating team");

    team_id
}

/// Create a bare `temper_next` profile and return its id. Inserting a profile
/// fires the substrate `sync_personal_team` / `sync_system_membership` triggers,
/// whose bodies reference UNQUALIFIED `kb_teams`/`kb_team_members`; run the
/// insert inside a `SET LOCAL search_path TO temper_next, public` txn (the
/// `read_selector::next_impl` idiom) so those resolve to the substrate.
async fn seed_profile(pool: &PgPool, handle: &str) -> Uuid {
    let id = Uuid::now_v7();
    let mut tx = pool.begin().await.expect("begin seed_profile txn");
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .expect("set search_path");
    sqlx::query(
        r#"INSERT INTO temper_next.kb_profiles (id, handle, display_name, email)
           VALUES ($1, $2, $2, $3)"#,
    )
    .bind(id)
    .bind(handle)
    .bind(format!("{handle}@example.test"))
    .execute(&mut *tx)
    .await
    .expect("seed profile");
    tx.commit().await.expect("commit seed_profile txn");
    id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn join_request_submit_inserts_pending_no_event(pool: PgPool) {
    let _team_id = arrange_invite_only(&pool, "gate-team").await;
    let requester = seed_profile(&pool, "requester-1").await;

    let request = access_service::create_join_request_next(
        &pool,
        CreateJoinRequestParams {
            profile_id: requester,
            message: Some("please let me in".to_string()),
            source: "web".to_string(),
            accepted_terms_version: None,
        },
    )
    .await
    .expect("create_join_request_next");

    assert_eq!(request.status, JoinRequestStatus::Pending);
    assert_eq!(request.requesting_profile_id, requester);

    // A pending row exists in the substrate operational table.
    let pending: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM temper_next.kb_join_requests
           WHERE requesting_profile_id = $1 AND status = 'pending'"#,
    )
    .bind(requester)
    .fetch_one(&pool)
    .await
    .expect("count pending requests");
    assert_eq!(pending, 1, "expected exactly one pending join request");

    // Admin events are firewalled from the cognition ledger: NO kb_events row.
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_events")
        .fetch_one(&pool)
        .await
        .expect("count cognition events");
    assert_eq!(
        events, 0,
        "join-request submission must NOT write the cognition ledger"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn review_request_approve_inserts_membership(pool: PgPool) {
    let team_id = arrange_invite_only(&pool, "gate-team").await;
    let requester = seed_profile(&pool, "requester-2").await;
    let reviewer = seed_profile(&pool, "reviewer-2").await;

    let request = access_service::create_join_request_next(
        &pool,
        CreateJoinRequestParams {
            profile_id: requester,
            message: None,
            source: "web".to_string(),
            accepted_terms_version: None,
        },
    )
    .await
    .expect("create_join_request_next");

    let reviewed = access_service::review_request_next(
        &pool,
        ReviewRequestParams {
            request_id: request.id,
            reviewer_profile_id: reviewer,
            decision: JoinRequestStatus::Approved,
            decision_note: None,
        },
    )
    .await
    .expect("review_request_next approve");

    assert_eq!(reviewed.status, JoinRequestStatus::Approved);
    assert_eq!(reviewed.reviewed_by_profile_id, Some(reviewer));

    // Approval inserts the substrate-shaped membership row (no id / joined_at).
    let role: String = sqlx::query_scalar(
        r#"SELECT role::text FROM temper_next.kb_team_members
           WHERE team_id = $1 AND profile_id = $2"#,
    )
    .bind(team_id)
    .bind(requester)
    .fetch_one(&pool)
    .await
    .expect("membership row exists");
    assert_eq!(role, "watcher");
}
