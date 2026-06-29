#![cfg(feature = "test-db")]
//! Integration tests for `context_service::resolve_context_ref`.
//!
//! Each test uses `#[sqlx::test]` with an isolated, seeded database. The
//! function under test is the single server-side resolver for context refs:
//! UUID-primary, `@me/slug`, `@handle/slug`, and `+team/slug` forms.
//!
//! Covers:
//! 1. `@me/slug` — resolves to the caller's own context
//! 2. Two same-name, distinct-slug contexts — each resolves distinctly (ambiguity regression)
//! 3. `+team-slug/slug` — member resolves; non-member gets `Forbidden`
//! 4. Bare UUID — visible resolves; not-visible gives `NotFound`
//! 5. `@handle/slug` — team-shared resolves; not-shared gives `NotFound`

mod common;

use sqlx::PgPool;
use temper_core::{context_ref::parse_context_ref, types::ids::ProfileId};
use temper_services::{error::ApiError, services::context_service};
use uuid::Uuid;

// ─── Fixture helpers ──────────────────────────────────────────────────────────

/// Create a team owned-context. Returns `(team_id, context_id)`.
async fn insert_team_with_context(pool: &PgPool, team_slug: &str, ctx_slug: &str) -> (Uuid, Uuid) {
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
    .expect("insert team context");

    (team_id, ctx_id)
}

/// Create a team without any context. Returns `team_id`.
async fn insert_team(pool: &PgPool, team_slug: &str) -> Uuid {
    let team_id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $3)")
        .bind(team_id)
        .bind(team_slug)
        .bind(team_slug)
        .execute(pool)
        .await
        .expect("insert team");
    team_id
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

// ─── Test 1: @me/slug resolves to the caller's own context ───────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resolves_at_me_slug_to_own_context(pool: PgPool) {
    let email = format!("me-slug-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let r = parse_context_ref("@me/temper").expect("valid ref");
    let result = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect("should resolve @me/temper to the profile's own context");

    assert_eq!(
        *result, context_id,
        "@me/temper should return the profile-owned context id"
    );
}

// ─── Test 2: two same-name, distinct-slug contexts resolve distinctly ─────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resolves_two_same_name_contexts_by_distinct_slug(pool: PgPool) {
    let email = format!("same-name-{}@example.com", Uuid::new_v4());
    let (profile_id, context_a_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    // The fixture creates a context with slug `temper` for the profile.
    // Insert a second context with the same name but a different slug —
    // the ambiguity-fix regression: name is NOT the resolution key.
    let context_b_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_profiles', $2, 'temper-2', 'temper')",
    )
    .bind(context_b_id)
    .bind(profile_id)
    .execute(&pool)
    .await
    .expect("insert second same-name context with distinct slug");

    // context A: slug 'temper'
    let r_a = parse_context_ref("@me/temper").expect("valid ref");
    let result_a = context_service::resolve_context_ref(&pool, principal, &r_a)
        .await
        .expect("should resolve @me/temper to context A");
    assert_eq!(*result_a, context_a_id, "@me/temper should give context A");

    // context B: slug 'temper-2'
    let r_b = parse_context_ref("@me/temper-2").expect("valid ref");
    let result_b = context_service::resolve_context_ref(&pool, principal, &r_b)
        .await
        .expect("should resolve @me/temper-2 to context B");
    assert_eq!(
        *result_b, context_b_id,
        "@me/temper-2 should give context B"
    );

    assert_ne!(*result_a, *result_b, "the two resolutions must be distinct");
}

// ─── Test 3a: +team-slug/slug resolves for a member ──────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resolves_team_context_for_member(pool: PgPool) {
    let email = format!("team-member-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let team_slug = format!("test-team-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (team_id, context_id) = insert_team_with_context(&pool, &team_slug, "notes").await;
    add_team_member(&pool, team_id, profile_id).await;

    let ref_str = format!("+{team_slug}/notes");
    let r = parse_context_ref(&ref_str).expect("valid team ref");
    let result = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect("team member should resolve team context");

    assert_eq!(*result, context_id);
}

// ─── Test 3b: +team-slug/slug gives Forbidden for non-members ────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_context_non_member_gets_forbidden(pool: PgPool) {
    let email = format!("team-nonmember-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let team_slug = format!("test-team-nm-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (_team_id, _context_id) = insert_team_with_context(&pool, &team_slug, "docs").await;
    // Deliberately do NOT add the profile as a member.

    let ref_str = format!("+{team_slug}/docs");
    let r = parse_context_ref(&ref_str).expect("valid team ref");
    let err = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect_err("non-member should not resolve team context");

    assert!(
        matches!(err, ApiError::Forbidden),
        "expected Forbidden for non-member, got {err:?}"
    );
}

// ─── Test 4a: bare UUID resolves when visible ─────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn bare_uuid_resolves_when_visible(pool: PgPool) {
    let email = format!("uuid-vis-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let r = parse_context_ref(&context_id.to_string()).expect("UUID is a valid ref");
    let result = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect("own context UUID should resolve");

    assert_eq!(*result, context_id);
}

// ─── Test 4b: bare UUID gives NotFound when not visible ──────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn bare_uuid_not_found_when_not_visible(pool: PgPool) {
    let email_a = format!("uuid-nv-a-{}@example.com", Uuid::new_v4());
    let (profile_a_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &email_a).await;

    let email_b = format!("uuid-nv-b-{}@example.com", Uuid::new_v4());
    let (_profile_b_id, context_b_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email_b).await;

    // Profile A tries to resolve Profile B's context UUID — not shared, should be invisible.
    let principal = ProfileId::from(profile_a_id);
    let r = parse_context_ref(&context_b_id.to_string()).expect("UUID is a valid ref");
    let err = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect_err("non-visible context UUID should not resolve");

    assert!(
        matches!(err, ApiError::NotFound),
        "expected NotFound for non-visible UUID, got {err:?}"
    );
}

// ─── Test 5a: @handle/slug resolves when context is team-shared ───────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn handle_slug_resolves_when_team_shared(pool: PgPool) {
    // Profile A is the principal; profile B owns the context being resolved.
    let email_a = format!("handle-vis-a-{}@example.com", Uuid::new_v4());
    let (profile_a_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &email_a).await;

    let email_b = format!("handle-vis-b-{}@example.com", Uuid::new_v4());
    let (profile_b_id, context_b_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email_b).await;

    // Reconstruct profile B's handle (mirrors the fixture formula).
    let b_local = email_b.split('@').next().unwrap_or("test");
    let b_handle = format!("{b_local}-{}", &profile_b_id.simple().to_string()[..8]);

    // Share B's `temper` context with a team that A is a member of.
    let team_slug = format!("shared-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let team_id = insert_team(&pool, &team_slug).await;
    add_team_member(&pool, team_id, profile_a_id).await;
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(context_b_id)
        .bind(team_id)
        .execute(&pool)
        .await
        .expect("share B's context with the team");

    let ref_str = format!("@{b_handle}/temper");
    let r = parse_context_ref(&ref_str).expect("valid @handle/slug ref");
    let principal = ProfileId::from(profile_a_id);
    let result = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect("A should resolve B's context via @handle/slug when team-shared");

    assert_eq!(
        *result, context_b_id,
        "@handle/slug should resolve to B's team-shared context"
    );
}

// ─── Test 5b: @handle/slug gives NotFound when context is not shared ──────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn handle_slug_not_found_when_not_shared(pool: PgPool) {
    let email_a = format!("handle-nv-a-{}@example.com", Uuid::new_v4());
    let (profile_a_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &email_a).await;

    let email_b = format!("handle-nv-b-{}@example.com", Uuid::new_v4());
    let (profile_b_id, _context_b_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email_b).await;

    let b_local = email_b.split('@').next().unwrap_or("test");
    let b_handle = format!("{b_local}-{}", &profile_b_id.simple().to_string()[..8]);

    // B's context is NOT shared with A — resolve should give NotFound.
    let ref_str = format!("@{b_handle}/temper");
    let r = parse_context_ref(&ref_str).expect("valid @handle/slug ref");
    let principal = ProfileId::from(profile_a_id);
    let err = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect_err("unshared @handle/slug should not resolve");

    assert!(
        matches!(err, ApiError::NotFound),
        "expected NotFound for unshared @handle/slug, got {err:?}"
    );
}
