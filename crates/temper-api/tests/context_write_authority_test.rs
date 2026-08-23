#![cfg(feature = "test-db")]
//! The context read answers whether the reader may author into it.
//!
//! A surface that offers a change must know, before offering, whether the reader holds the
//! authority to make it. `can_modify_resource`'s four arms are a floor plus owner, two per-resource
//! grant arms, and a **container-write cascade** that delegates to
//! `context_authorable_by_profile`. That last arm is per-CONTAINER, so one boolean answers every
//! resource homed there — which is why the answer belongs on the context read rather than on every
//! response that carries a resource.
//!
//! The discriminating case is a **`watcher`**. Migration `20260712000010` narrowed the team arm of
//! `context_authorable_by_profile` to DIRECT membership with an authoring role, and its own
//! `COMMENT ON FUNCTION` states the property these tests pin: *"watcher is read-only"*. Read is
//! strictly broader than write, so a watcher reads the context and must not be told they may write
//! it. A `can_write` that merely tracked visibility would be true for every row and would pass any
//! test that did not seed this shape.
//!
//! The last test pins the property that makes carrying authority here safe at all: the value is
//! computed INSIDE the read-gated query, so a context the reader cannot read is **absent** (list)
//! or the one shared refusal (get) — never a row saying `can_write: false`, which would confirm
//! existence to someone with no standing to ask.

mod common;

use sqlx::PgPool;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::{error::ApiError, services::context_service};
use uuid::Uuid;

// ─── Fixture helpers ──────────────────────────────────────────────────────────

/// A team that OWNS a context. Returns `(team_id, context_id)`.
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

    (team_id, ctx_id)
}

/// Add a profile to a team in a named role. `watcher` is read-only; `member` authors.
async fn add_team_member(pool: &PgPool, team_id: Uuid, profile_id: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team_id)
    .bind(profile_id)
    .bind(role)
    .execute(pool)
    .await
    .expect("add team member");
}

fn team_slug() -> String {
    format!("wa-team-{}", &Uuid::new_v4().simple().to_string()[..8])
}

// ─── The reader who owns their own context may author into it ─────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn personal_context_owner_may_author(pool: PgPool) {
    let email = format!("wa-own-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let row = context_service::get_visible(&pool, principal, ContextId::from(context_id))
        .await
        .expect("owner reads their own context");

    assert!(
        row.can_write,
        "the owner of a personal-owned context authors it (context_authorable_by_profile's \
         personal-owned arm)"
    );
}

// ─── THE DISCRIMINATING CASE: a watcher reads and must not be told they may write ─

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_watcher_reads_but_may_not_author(pool: PgPool) {
    let email = format!("wa-watch-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let (team_id, context_id) = insert_team_owned_context(&pool, &team_slug(), "watched").await;
    add_team_member(&pool, team_id, profile_id, "watcher").await;

    // Read reaches it — that is the half that must NOT be mistaken for write.
    let row = context_service::get_visible(&pool, principal, ContextId::from(context_id))
        .await
        .expect("a watcher READS the team-owned context");
    assert_eq!(*row.id, context_id, "precondition: the watcher can read it");

    assert!(
        !row.can_write,
        "`watcher` is read-only (migration 20260712000010's COMMENT ON FUNCTION says so). A \
         can_write that tracked visibility rather than authority would be true here"
    );

    // Same property through the list door, which is what a nav actually reads.
    let rows = context_service::list_visible(&pool, principal)
        .await
        .expect("list_visible succeeds for the watcher");
    let listed = rows
        .iter()
        .find(|r| *r.id == context_id)
        .expect("the watched context is visible in the list");
    assert!(
        !listed.can_write,
        "the list door must answer authority the same way the get door does"
    );
}

// ─── An authoring role may author ─────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_member_may_author(pool: PgPool) {
    let email = format!("wa-member-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let (team_id, context_id) = insert_team_owned_context(&pool, &team_slug(), "authored").await;
    add_team_member(&pool, team_id, profile_id, "member").await;

    let rows = context_service::list_visible(&pool, principal)
        .await
        .expect("list_visible succeeds for the member");
    let listed = rows
        .iter()
        .find(|r| *r.id == context_id)
        .expect("the team context is visible to its member");

    assert!(
        listed.can_write,
        "`member` is an authoring role, so the container-write cascade reaches every resource \
         homed here"
    );
}

// ─── The authority answer never reveals a context the reader cannot read ──────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unreadable_context_is_absent_not_reported_unauthorized(pool: PgPool) {
    let stranger_email = format!("wa-stranger-{}@example.com", Uuid::new_v4());
    let (stranger_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &stranger_email).await;
    let stranger = ProfileId::from(stranger_id);

    // A team context the stranger has no membership in and no grant on.
    let (_team_id, context_id) = insert_team_owned_context(&pool, &team_slug(), "private").await;

    // The get door: the one shared refusal, NOT a row carrying `can_write: false`.
    let err = context_service::get_visible(&pool, stranger, ContextId::from(context_id))
        .await
        .expect_err("a context the stranger cannot read must not be returned at all");
    assert!(
        matches!(err, ApiError::NotFound(_)),
        "an unreadable context must be indistinguishable from a nonexistent one; got {err:?}"
    );

    // The list door: absent, not present-and-false.
    let rows = context_service::list_visible(&pool, stranger)
        .await
        .expect("list_visible succeeds for the stranger");
    assert!(
        !rows.iter().any(|r| *r.id == context_id),
        "carrying authority on the context read must not add a row for a context the reader \
         cannot read"
    );
}
