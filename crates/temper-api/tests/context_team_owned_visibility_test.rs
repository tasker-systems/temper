#![cfg(feature = "test-db")]
//! Integration tests for the I2 fix: a TEAM-OWNED context
//! (`owner_table='kb_teams'`) with **no** `kb_team_contexts` self-share row must
//! be visible to a member of the owning team via every context-visibility path
//! (UUID resolve + `list_visible`), exactly as it already was via `+team/slug`.
//!
//! Before the fix, `context_visible_to` did not exist and the resolve/list/get
//! sites admitted a context only when profile-owned OR explicitly shared via
//! `kb_team_contexts`. A team-owned context with no self-share row therefore
//! resolved by `+team/slug` but was `NotFound` by UUID and absent from `list`.
//!
//! These tests seed exactly that shape — a team, a membership, and a team-owned
//! context with NO share row — and assert:
//! 1. Member resolves the team-owned context by its bare UUID (was `NotFound`).
//! 2. The team-owned context appears in the member's `list_visible`.
//! 3. A non-member gets `NotFound` by UUID and the context is absent from their
//!    `list_visible` (no false-positive / no leak).
//! 4. `+team/slug` still resolves for the member and is still `Forbidden` for the
//!    non-member (regression guard for the path that was already correct).

mod common;

use sqlx::PgPool;
use temper_api::{error::ApiError, services::context_service};
use temper_core::{context_ref::parse_context_ref, types::ids::ProfileId};
use uuid::Uuid;

// ─── Fixture helpers ──────────────────────────────────────────────────────────

/// Create a team that OWNS a context, with NO `kb_team_contexts` self-share row.
/// This is the exact shape that exhibited the I2 false-negative.
/// Returns `(team_id, context_id)`.
async fn insert_team_owned_context(pool: &PgPool, team_slug: &str, ctx_slug: &str) -> (Uuid, Uuid) {
    let team_id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $3)")
        .bind(team_id)
        .bind(team_slug)
        .bind(team_slug)
        .execute(pool)
        .await
        .expect("insert team");

    let ctx_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_teams', $2, $3, $4)",
    )
    .bind(ctx_id)
    .bind(team_id)
    .bind(ctx_slug)
    .bind(ctx_slug)
    .execute(pool)
    .await
    .expect("insert team-owned context");

    // Deliberately NO `kb_team_contexts` row — the no-self-share shape is the
    // whole point of the I2 regression.
    (team_id, ctx_id)
}

/// Add a profile as a `member` of a team.
async fn add_team_member(pool: &PgPool, team_id: Uuid, profile_id: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("add team member");
}

// ─── Test 1: member resolves a team-owned context by bare UUID ────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn member_resolves_team_owned_context_by_uuid(pool: PgPool) {
    let email = format!("to-uuid-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let team_slug = format!("to-team-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (team_id, context_id) = insert_team_owned_context(&pool, &team_slug, "notes").await;
    add_team_member(&pool, team_id, profile_id).await;

    // The crux of I2: resolve the team-owned context by its bare UUID. This was
    // `NotFound` before the fix even though `+team/slug` resolved it.
    let r = parse_context_ref(&context_id.to_string()).expect("UUID is a valid ref");
    let result = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect("member should resolve team-owned context by UUID");

    assert_eq!(
        *result, context_id,
        "team member must resolve the team-owned context by its UUID"
    );
}

// ─── Test 2: team-owned context appears in member's list_visible ──────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn member_lists_team_owned_context(pool: PgPool) {
    let email = format!("to-list-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let team_slug = format!("to-team-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (team_id, context_id) = insert_team_owned_context(&pool, &team_slug, "docs").await;
    add_team_member(&pool, team_id, profile_id).await;

    let rows = context_service::list_visible(&pool, principal)
        .await
        .expect("list_visible must succeed for the member");

    assert!(
        rows.iter().any(|r| *r.id == context_id),
        "team-owned context must appear in the member's list_visible; got: {:?}",
        rows.iter().map(|r| *r.id).collect::<Vec<_>>()
    );
}

// ─── Test 3: non-member gets NotFound by UUID and does not see it in list ─────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_member_cannot_see_team_owned_context(pool: PgPool) {
    // The principal is NOT a member of the owning team.
    let email = format!("to-nonmember-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let team_slug = format!("to-team-nm-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (_team_id, context_id) = insert_team_owned_context(&pool, &team_slug, "secret").await;
    // Deliberately do NOT add the principal as a member.

    // 3a. UUID resolve → NotFound (no leak).
    let r = parse_context_ref(&context_id.to_string()).expect("UUID is a valid ref");
    let err = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect_err("non-member must not resolve a team-owned context by UUID");
    assert!(
        matches!(err, ApiError::NotFound),
        "expected NotFound for non-member UUID resolve, got {err:?}"
    );

    // 3b. Absent from list_visible (no leak).
    let rows = context_service::list_visible(&pool, principal)
        .await
        .expect("list_visible must succeed");
    assert!(
        !rows.iter().any(|r| *r.id == context_id),
        "team-owned context must NOT appear in a non-member's list_visible"
    );
}

// ─── Test 4: +team/slug regression guard (member resolves; non-member Forbidden)

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_slug_path_still_correct(pool: PgPool) {
    // Member.
    let email_m = format!("to-ts-m-{}@example.com", Uuid::new_v4());
    let (member_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email_m).await;
    let member = ProfileId::from(member_id);

    // Non-member.
    let email_nm = format!("to-ts-nm-{}@example.com", Uuid::new_v4());
    let (nonmember_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &email_nm).await;
    let nonmember = ProfileId::from(nonmember_id);

    let team_slug = format!("to-ts-team-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (team_id, context_id) = insert_team_owned_context(&pool, &team_slug, "plans").await;
    add_team_member(&pool, team_id, member_id).await;

    let ref_str = format!("+{team_slug}/plans");
    let r = parse_context_ref(&ref_str).expect("valid team ref");

    // Member resolves.
    let result = context_service::resolve_context_ref(&pool, member, &r)
        .await
        .expect("member should resolve +team/slug");
    assert_eq!(
        *result, context_id,
        "+team/slug must resolve for the member"
    );

    // Non-member is Forbidden (existence not leaked as NotFound).
    let err = context_service::resolve_context_ref(&pool, nonmember, &r)
        .await
        .expect_err("non-member must not resolve +team/slug");
    assert!(
        matches!(err, ApiError::Forbidden),
        "expected Forbidden for non-member +team/slug, got {err:?}"
    );
}
