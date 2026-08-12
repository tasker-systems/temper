#![cfg(feature = "test-db")]
//! Team-context creation (org-provisioning Chunk 3) — owner-parameterized
//! `context_service::create` plus the `resolve_create_owner` role gate.
//!
//! Gating (spec §4 Chunk 3): a context owned by a team requires the caller to be
//! `owner`/`maintainer` on that team (reuses Chunk 2's `team_service` role check);
//! a plain `member`/non-member is `Forbidden`; an `@handle` owner is `BadRequest`
//! (a profile cannot create a context it does not own); `None`/`@me` preserves the
//! pre-Chunk-3 profile-owned behavior, including owner-scoped slug auto-suffixing.

mod common;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::context_ref::{parse_context_ref, ContextOwnerRef};
use temper_core::types::context::ContextRow;
use temper_core::types::ids::ProfileId;
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};
use temper_services::error::ApiError;
use temper_services::services::{context_service, team_service};

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Mirror the handler: resolve the owner (role-gated) then create.
async fn create_for_owner(
    pool: &PgPool,
    caller: Uuid,
    owner: Option<ContextOwnerRef>,
    name: &str,
) -> Result<ContextRow, ApiError> {
    let (owner_table, owner_id) =
        context_service::resolve_create_owner(pool, ProfileId::from(caller), owner.as_ref())
            .await?;
    context_service::create(pool, &owner_table, owner_id, name).await
}

/// Create a root team owned by `owner`. Returns the team id.
async fn create_team(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    team_service::create_team(
        pool,
        ProfileId::from(owner),
        &TeamCreateRequest {
            slug: slug.to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        },
    )
    .await
    .expect("create team")
    .id
}

/// Soft-delete a team, the way `DELETE /api/teams/{id}` does: flip `is_active`, preserve every row
/// hanging off it (members, contexts). A bare UPDATE deliberately — the assertion is about what the
/// *create* path does with the flag, so the test must not be able to pass because a service helper
/// also cleaned up the caller's membership.
async fn soft_delete_team(pool: &PgPool, team_id: Uuid) {
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(team_id)
        .execute(pool)
        .await
        .expect("soft-delete team");
}

/// Add `profile` to `team` at `role` (acting as `actor`, who must be owner/maintainer).
async fn add_member(pool: &PgPool, actor: Uuid, team_id: Uuid, profile: Uuid, role: TeamRole) {
    team_service::add_member(
        pool,
        ProfileId::from(actor),
        team_id,
        &AddMemberRequest {
            profile_id: profile,
            role,
        },
    )
    .await
    .expect("add member");
}

// ─── owner / maintainer may create a team-owned context ──────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn owner_creates_team_context(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "ctx-owner@example.com").await;
    let team_id = create_team(&pool, owner, "ctx-team-owner").await;

    let row = create_for_owner(
        &pool,
        owner,
        Some(ContextOwnerRef::Team("ctx-team-owner".to_owned())),
        "shared docs",
    )
    .await
    .expect("owner may create a team-owned context");

    assert_eq!(row.kb_owner_table, "kb_teams");
    assert_eq!(row.kb_owner_id, team_id);
    assert_eq!(row.owner_ref, "+ctx-team-owner");
    assert_eq!(row.slug, "shared-docs");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn maintainer_creates_team_context(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "mc-owner@example.com").await;
    let maintainer = common::fixtures::create_test_profile(&pool, "mc-maint@example.com").await;
    let team_id = create_team(&pool, owner, "mc-team").await;
    add_member(&pool, owner, team_id, maintainer, TeamRole::Maintainer).await;

    let row = create_for_owner(
        &pool,
        maintainer,
        Some(ContextOwnerRef::Team("mc-team".to_owned())),
        "notes",
    )
    .await
    .expect("maintainer may create a team-owned context");

    assert_eq!(row.kb_owner_table, "kb_teams");
    assert_eq!(row.kb_owner_id, team_id);
}

// ─── member / non-member → Forbidden ─────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn plain_member_is_forbidden(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "pm-owner@example.com").await;
    let member = common::fixtures::create_test_profile(&pool, "pm-member@example.com").await;
    let team_id = create_team(&pool, owner, "pm-team").await;
    add_member(&pool, owner, team_id, member, TeamRole::Member).await;

    let denied = create_for_owner(
        &pool,
        member,
        Some(ContextOwnerRef::Team("pm-team".to_owned())),
        "notes",
    )
    .await;

    assert!(
        matches!(denied, Err(ApiError::Forbidden)),
        "plain member must be Forbidden, got {denied:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_member_is_forbidden(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "nm-owner@example.com").await;
    let stranger = common::fixtures::create_test_profile(&pool, "nm-stranger@example.com").await;
    let _team_id = create_team(&pool, owner, "nm-team").await;

    let denied = create_for_owner(
        &pool,
        stranger,
        Some(ContextOwnerRef::Team("nm-team".to_owned())),
        "notes",
    )
    .await;

    assert!(
        matches!(denied, Err(ApiError::Forbidden)),
        "non-member must be Forbidden, got {denied:?}"
    );
}

// ─── @handle owner → BadRequest ──────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn handle_owner_is_bad_request(pool: PgPool) {
    let caller = common::fixtures::create_test_profile(&pool, "ho-caller@example.com").await;

    let denied = create_for_owner(
        &pool,
        caller,
        Some(ContextOwnerRef::Handle("someone-else".to_owned())),
        "notes",
    )
    .await;

    assert!(
        matches!(denied, Err(ApiError::BadRequest(_))),
        "creating a context owned by another profile must be BadRequest, got {denied:?}"
    );
}

// ─── unknown team → NotFound ─────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unknown_team_is_not_found(pool: PgPool) {
    let caller = common::fixtures::create_test_profile(&pool, "ut-caller@example.com").await;

    let denied = create_for_owner(
        &pool,
        caller,
        Some(ContextOwnerRef::Team("no-such-team".to_owned())),
        "notes",
    )
    .await;

    assert!(
        matches!(denied, Err(ApiError::NotFound(_))),
        "unknown team must be NotFound, got {denied:?}"
    );
}

// ─── a soft-deleted team may not own a NEW context ───────────────────────────
//
// `20260703000001_team_metadata_soft_delete.sql` declares in its header that "membership in a
// soft-deleted team confers nothing anywhere". The SQL read paths honour it at two chokepoints;
// `resolve_create_owner` resolves the team in Rust and is the WRITE-side twin of
// `resolve_context_ref`'s team arm — without the `is_active` filter an owner of a dead team could
// mint a context OWNED by it, i.e. state every read path (this one included) then refuses to show
// anyone, its creator among them.
//
// The refusal must be the NOT-FOUND one and must read exactly like a team that never existed.
// `Forbidden` would be wrong twice over: it discloses that the slug names something, and it is
// false — this caller is the team's owner.

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn soft_deleted_team_cannot_own_a_new_context(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "sdc-owner@example.com").await;
    let team_id = create_team(&pool, owner, "sdc-team").await;

    // (a) While the team is ACTIVE its owner creates a team-owned context — the before half of the
    //     pair, so a later regression cannot make this test pass by breaking create outright.
    let row = create_for_owner(
        &pool,
        owner,
        Some(ContextOwnerRef::Team("sdc-team".to_owned())),
        "plans",
    )
    .await
    .expect("owner of an active team may create a team-owned context");
    assert_eq!(row.kb_owner_table, "kb_teams");
    assert_eq!(row.kb_owner_id, team_id);

    // (b) Soft-delete. The owner's `kb_team_members` row survives untouched, so the refusal below
    //     can only be coming from `is_active`.
    soft_delete_team(&pool, team_id).await;

    let err = create_for_owner(
        &pool,
        owner,
        Some(ContextOwnerRef::Team("sdc-team".to_owned())),
        "more plans",
    )
    .await
    .expect_err("a soft-deleted team must not be able to own a new context");
    let ApiError::NotFound(msg) = err else {
        panic!("expected NotFound for a soft-deleted team, got {err:?} (Forbidden would both leak the team's existence and be false — this caller is its owner)");
    };

    // And it must be indistinguishable from a team that was never created at all.
    let absent = create_for_owner(
        &pool,
        owner,
        Some(ContextOwnerRef::Team("sdc-absent".to_owned())),
        "plans",
    )
    .await
    .expect_err("a nonexistent team must not own a context");
    let ApiError::NotFound(absent_msg) = absent else {
        panic!("expected NotFound for a nonexistent team, got {absent:?}");
    };
    assert_eq!(
        msg.replace("sdc-team", "<slug>"),
        absent_msg.replace("sdc-absent", "<slug>"),
        "a soft-deleted team must refuse in the same words as one that never existed"
    );
}

// ─── profile-owned create (owner None) + owner-scoped slug suffix ─────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn profile_owned_create_still_works_and_suffixes_per_owner(pool: PgPool) {
    let caller = common::fixtures::create_test_profile(&pool, "po-caller@example.com").await;

    // Two same-named contexts under the SAME owner → distinct slugs (auto-suffix).
    let a = create_for_owner(&pool, caller, None, "My Notes")
        .await
        .expect("first profile-owned create");
    let b = create_for_owner(&pool, caller, None, "My Notes")
        .await
        .expect("second profile-owned create");

    assert_eq!(a.kb_owner_table, "kb_profiles");
    assert_eq!(a.kb_owner_id, caller);
    assert_eq!(a.slug, "my-notes");
    assert_eq!(
        b.slug, "my-notes-2",
        "same-owner collision must auto-suffix"
    );

    // Same name under a DIFFERENT owner (a team) keeps the base slug — the
    // collision check is owner-scoped, not global.
    let owner = common::fixtures::create_test_profile(&pool, "po-team-owner@example.com").await;
    create_team(&pool, owner, "po-team").await;
    let t = create_for_owner(
        &pool,
        owner,
        Some(ContextOwnerRef::Team("po-team".to_owned())),
        "My Notes",
    )
    .await
    .expect("team-owned create with same name");
    assert_eq!(
        t.slug, "my-notes",
        "a different owner may keep the base slug"
    );
}

// ─── a team member can resolve the freshly-created team context ──────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn member_can_resolve_created_team_context(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "rc-owner@example.com").await;
    let member = common::fixtures::create_test_profile(&pool, "rc-member@example.com").await;
    let team_id = create_team(&pool, owner, "rc-team").await;
    add_member(&pool, owner, team_id, member, TeamRole::Member).await;

    let created = create_for_owner(
        &pool,
        owner,
        Some(ContextOwnerRef::Team("rc-team".to_owned())),
        "team docs",
    )
    .await
    .expect("owner creates team context");

    let r = parse_context_ref("+rc-team/team-docs").expect("valid team ref");
    let resolved = context_service::resolve_context_ref(&pool, ProfileId::from(member), &r)
        .await
        .expect("member resolves the freshly-created team context");

    assert_eq!(
        *resolved, *created.id,
        "member must resolve to the new context"
    );
}

// ─── HTTP-level: owner creates a team context via POST /api/contexts ─────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn http_owner_creates_team_context(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("http-ctx-owner-{}@example.com", Uuid::new_v4());
    let owner = common::fixtures::create_test_profile(&app.pool, &email).await;
    create_team(&app.pool, owner, "http-ctx-team").await;
    let token = common::generate_test_jwt(&format!("test|{owner}"), &email);

    let resp = app
        .client
        .post(app.url("/api/contexts"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "name": "shared", "owner": { "Team": "http-ctx-team" } }))
        .send()
        .await
        .expect("create context request");

    assert_eq!(
        resp.status(),
        201,
        "owner creating a team context must be 201"
    );
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["kb_owner_table"], "kb_teams");
    assert_eq!(body["owner_ref"], "+http-ctx-team");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn http_non_member_team_context_is_403(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let owner = common::fixtures::create_test_profile(&app.pool, "http-cm-owner@example.com").await;
    create_team(&app.pool, owner, "http-cm-team").await;

    let email = format!("http-cm-stranger-{}@example.com", Uuid::new_v4());
    let stranger = common::fixtures::create_test_profile(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{stranger}"), &email);

    let resp = app
        .client
        .post(app.url("/api/contexts"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "name": "shared", "owner": { "Team": "http-cm-team" } }))
        .send()
        .await
        .expect("create context request");

    assert_eq!(
        resp.status(),
        403,
        "non-member creating a team context must be 403"
    );
}
