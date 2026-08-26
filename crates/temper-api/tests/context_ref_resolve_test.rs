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
//! 6. A RETIRED context resolves through no arm — not `@me/slug` for its own owner, and not
//!    `+team/slug` for a member. Every caller documents this resolver as visibility-gated, so it
//!    must agree with `context_visible_to`, which refuses a retired context for every principal.

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
        matches!(err, ApiError::NotFound(_)),
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
        matches!(err, ApiError::NotFound(_)),
        "expected NotFound for unshared @handle/slug, got {err:?}"
    );
}

// ─── Test 5c: the three @handle/slug refusals are one refusal ────────────────

/// A caller probing `@handle/slug` must not be able to tell **which** of three things went wrong.
///
/// The arm has three failure exits, and at base every one of them rendered the bare string
/// `Not found`, so they were indistinguishable for free. Once `ApiError::NotFound` carries a
/// message that is no longer free, and each exit will happily describe itself:
///
/// * the handle does not resolve to a profile,
/// * it does, but that profile owns no context by this slug,
/// * it does own one, and the caller may not see it.
///
/// Told apart, those three answer questions the caller has no standing to ask. The first is a
/// membership oracle over every handle on the instance. The second and third are worse together:
/// one guess per request, and a caller enumerates the **private context slugs of another user**,
/// learning for each guess whether it named a real context — which is exactly what "not found or
/// not readable" exists to withhold.
///
/// So this asserts byte-identity, not a shared variant. Status is no longer the whole signal, and
/// the two sibling tests above (`bare_uuid_not_found_when_not_visible`,
/// `handle_slug_not_found_when_not_shared`) check only `matches!(err, ApiError::NotFound(_))` —
/// which a tuple variant satisfies no matter what string it carries. They were written when the
/// variant *was* the whole signal and cannot catch this.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_three_handle_slug_refusals_are_indistinguishable(pool: PgPool) {
    let email_a = format!("oracle-a-{}@example.com", Uuid::new_v4());
    let (profile_a_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &email_a).await;

    let email_b = format!("oracle-b-{}@example.com", Uuid::new_v4());
    let (profile_b_id, _context_b_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email_b).await;

    let b_local = email_b.split('@').next().unwrap_or("test");
    let b_handle = format!("{b_local}-{}", &profile_b_id.simple().to_string()[..8]);
    let principal = ProfileId::from(profile_a_id);

    /// Resolve a ref expected to fail, and return the rendered refusal.
    async fn refusal(pool: &PgPool, principal: ProfileId, ref_str: &str) -> String {
        let r = parse_context_ref(ref_str).expect("valid ref");
        match context_service::resolve_context_ref(pool, principal, &r).await {
            Err(ApiError::NotFound(msg)) => msg,
            other => panic!("{ref_str} must refuse with NotFound; got {other:?}"),
        }
    }

    // The fixture gives B a context named `temper`, and nothing shares it with A.
    let absent_handle = refusal(&pool, principal, "@no-such-handle-at-all/temper").await;
    let absent_slug = refusal(&pool, principal, &format!("@{b_handle}/no-such-context")).await;
    let unreadable = refusal(&pool, principal, &format!("@{b_handle}/temper")).await;

    assert_eq!(
        [&absent_handle, &absent_slug, &unreadable]
            .map(String::as_str)
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1,
        "all three @handle/slug refusals must be byte-identical: \
         absent-handle {absent_handle:?}, absent-slug {absent_slug:?}, unreadable {unreadable:?}"
    );
}

// ─── Test 6: a retired context resolves through no arm ───────────────────────
//
// Both arms below read `kb_contexts` by `(owner, slug)`. Neither consulted the read predicate
// before this change, so each resolved a context `context_visible_to` refuses.

/// Retire a context the way the service does, without depending on `context_service::retire` —
/// this file tests the resolver, not the verb, and the mangled slug is what an operator would
/// actually hold afterwards.
async fn retire(pool: &PgPool, context_id: Uuid, retired_slug: &str) {
    sqlx::query("UPDATE kb_contexts SET is_active = false, slug = $2 WHERE id = $1")
        .bind(context_id)
        .bind(retired_slug)
        .execute(pool)
        .await
        .expect("retire the context");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retired_context_does_not_resolve_by_at_me_slug_even_for_its_owner(pool: PgPool) {
    let email = format!("me-retired-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    // Precondition: it resolves while active, so the refusal below is the retirement and not a
    // broken fixture.
    let live = parse_context_ref("@me/temper").expect("valid ref");
    assert_eq!(
        *context_service::resolve_context_ref(&pool, principal, &live)
            .await
            .expect("resolves while active"),
        context_id
    );

    retire(&pool, context_id, "temper-retired").await;

    let r = parse_context_ref("@me/temper-retired").expect("valid ref");
    let err = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect_err("a retired context is not addressable on the read axis, even by its owner");
    assert!(
        matches!(err, ApiError::NotFound(_)),
        "expected NotFound for a retired @me context, got {err:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retired_team_context_does_not_resolve_for_a_member(pool: PgPool) {
    let email = format!("team-retired-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let team_slug = format!("test-team-rt-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (team_id, context_id) = insert_team_with_context(&pool, &team_slug, "notes").await;
    add_team_member(&pool, team_id, profile_id).await;

    // Precondition: membership resolves it while the context is active.
    let live_ref = format!("+{team_slug}/notes");
    let live = parse_context_ref(&live_ref).expect("valid team ref");
    assert_eq!(
        *context_service::resolve_context_ref(&pool, principal, &live)
            .await
            .expect("member resolves while active"),
        context_id
    );

    retire(&pool, context_id, "notes-retired").await;

    // Membership is unchanged and still admits; only visibility refuses. Without the gate the
    // member — at ANY role, `watcher` included — still resolves the retired context.
    let ref_str = format!("+{team_slug}/notes-retired");
    let r = parse_context_ref(&ref_str).expect("valid team ref");
    let err = context_service::resolve_context_ref(&pool, principal, &r)
        .await
        .expect_err("membership is not visibility: a retired team context must refuse");
    assert!(
        matches!(err, ApiError::NotFound(_)),
        "expected NotFound for a retired team context, got {err:?}"
    );
}

/// For ONE slug, "it is retired" and "it never existed" must be indistinguishable.
///
/// The comparison has to hold the SLUG constant and vary existence. An earlier draft of this test
/// compared the refusals for two *different* slugs and failed — correctly: each refusal echoes the
/// slug the caller supplied, so different inputs give different strings, and that difference
/// discloses nothing. The oracle is narrower: ask for one name and learn from the answer whether a
/// retired context sits behind it.
///
/// The pair above (`retired_context_does_not_resolve_*`) asserts only `matches!(err, NotFound(_))`,
/// which a tuple variant satisfies whatever string it carries — the same blind spot
/// `the_three_handle_slug_refusals_are_indistinguishable` was written for. Gating an arm with a
/// bare refusal while its miss names the slug would reopen exactly this.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn one_slug_refuses_alike_whether_retired_or_absent(pool: PgPool) {
    async fn refusal(pool: &PgPool, principal: ProfileId, ref_str: &str) -> String {
        let r = parse_context_ref(ref_str).expect("valid ref");
        match context_service::resolve_context_ref(pool, principal, &r).await {
            Err(ApiError::NotFound(msg)) => msg,
            other => panic!("{ref_str} must refuse with NotFound; got {other:?}"),
        }
    }

    // ── @me: same ref, one principal where it is retired, one where it is absent ──
    let email_r = format!("parity-retired-{}@example.com", Uuid::new_v4());
    let (retired_owner, retired_ctx) =
        common::fixtures::create_test_profile_with_context(&pool, &email_r).await;
    retire(&pool, retired_ctx, "temper-retired").await;

    let email_a = format!("parity-absent-{}@example.com", Uuid::new_v4());
    let (absent_owner, _) =
        common::fixtures::create_test_profile_with_context(&pool, &email_a).await;

    let me_retired = refusal(&pool, ProfileId::from(retired_owner), "@me/temper-retired").await;
    let me_absent = refusal(&pool, ProfileId::from(absent_owner), "@me/temper-retired").await;
    assert_eq!(
        me_retired, me_absent,
        "@me/temper-retired must read alike whether the context is retired or was never there — \
         retired {me_retired:?}, absent {me_absent:?}"
    );

    // ── +team: same ref shape against two teams the caller belongs to ─────────
    let member = absent_owner;
    let t_retired = format!("parity-rt-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (retired_team, team_ctx) = insert_team_with_context(&pool, &t_retired, "notes").await;
    add_team_member(&pool, retired_team, member).await;
    retire(&pool, team_ctx, "notes-retired").await;

    let t_absent = format!("parity-ab-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (absent_team, _) = insert_team_with_context(&pool, &t_absent, "notes").await;
    add_team_member(&pool, absent_team, member).await;

    let principal = ProfileId::from(member);
    let team_retired = refusal(&pool, principal, &format!("+{t_retired}/notes-retired")).await;
    let team_absent = refusal(&pool, principal, &format!("+{t_absent}/notes-retired")).await;
    assert_eq!(
        team_retired, team_absent,
        "+team/notes-retired must read alike whether retired or never there — \
         retired {team_retired:?}, absent {team_absent:?}"
    );
}
