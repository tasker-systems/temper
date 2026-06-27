#![cfg(feature = "test-db")]
//! Integration tests for the team-owned-context **resource** visibility fix
//! (follow-up to the context-ref arc's I2 fix).
//!
//! The I2 fix (`context_visible_to`, migration `20260627000001`) made a
//! TEAM-OWNED context (`owner_table='kb_teams'`) with **no** `kb_team_contexts`
//! self-share row addressable/listable by a member of the owning team. But the
//! *resource*-visibility predicates — `resources_visible_to` and
//! `anchor_readable_by_profile` — still gated team access on a `kb_team_contexts`
//! share row and lacked the "homed in a context owned by a team I'm a member of"
//! clause. So a member could address a team-owned context yet NOT see the
//! resources inside it.
//!
//! These tests seed exactly that shape — a team, two members, a team-owned
//! context with NO share row, and a resource homed in it owned by a DIFFERENT
//! member — and assert:
//! 1. A second team member (who does not own the resource) sees it via
//!    `resources_visible_to` (the new team-ownership clause, not the owner clause).
//! 2. `anchor_readable_by_profile` admits the team-owned context for that member.
//! 3. A non-member sees neither the resource nor the context anchor (no leak).
//!
//! The resource owner is deliberately a *different* profile than the principal
//! under test, so the owner/originator clause of `resources_visible_to` cannot
//! mask a regression in the team-ownership clause.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

// ─── Fixture helpers ──────────────────────────────────────────────────────────

/// Create a team that OWNS a context, with NO `kb_team_contexts` self-share row.
/// This is the exact shape that exhibited the resource-visibility false-negative.
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
    // whole point of the regression.
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

/// Home a resource in `context_id`, owned/originated by `owner_id`. Returns the
/// resource id.
async fn home_resource_in_context(
    pool: &PgPool,
    context_id: Uuid,
    owner_id: Uuid,
    title: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(title)
        .bind(format!("test://{id}"))
        .execute(pool)
        .await
        .expect("insert resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
            (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(context_id)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("home resource");
    id
}

/// `true` when `resources_visible_to(profile)` includes `resource_id`.
async fn resource_visible(pool: &PgPool, profile_id: Uuid, resource_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(profile_id)
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("resources_visible_to query")
}

/// `anchor_readable_by_profile(profile, 'kb_contexts', context_id)`.
async fn context_anchor_readable(pool: &PgPool, profile_id: Uuid, context_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT anchor_readable_by_profile($1, 'kb_contexts', $2)")
        .bind(profile_id)
        .bind(context_id)
        .fetch_one(pool)
        .await
        .expect("anchor_readable_by_profile query")
}

// ─── Test 1: a non-owner team member sees a resource in the team-owned context ─

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn member_sees_resource_in_team_owned_context(pool: PgPool) {
    // The resource OWNER (a first member).
    let owner_email = format!("tor-owner-{}@example.com", Uuid::new_v4());
    let (owner_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &owner_email).await;

    // The principal under test (a SECOND member who does not own the resource).
    let member_email = format!("tor-member-{}@example.com", Uuid::new_v4());
    let (member_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &member_email).await;

    let team_slug = format!("tor-team-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (team_id, context_id) = insert_team_owned_context(&pool, &team_slug, "notes").await;
    add_team_member(&pool, team_id, owner_id).await;
    add_team_member(&pool, team_id, member_id).await;

    let resource_id = home_resource_in_context(&pool, context_id, owner_id, "Team note").await;

    // The crux: the second member sees the resource purely via the team-ownership
    // clause (they neither own it nor have a `kb_team_contexts` share row).
    assert!(
        resource_visible(&pool, member_id, resource_id).await,
        "team member must see a resource homed in their team-owned context"
    );
    assert!(
        context_anchor_readable(&pool, member_id, context_id).await,
        "team member must read the team-owned context anchor"
    );
}

// ─── Test 2: a non-member sees neither the resource nor the context anchor ─────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_member_cannot_see_resource_in_team_owned_context(pool: PgPool) {
    // The resource OWNER + team member.
    let owner_email = format!("tor-nm-owner-{}@example.com", Uuid::new_v4());
    let (owner_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &owner_email).await;

    // The principal under test — NOT a member of the owning team.
    let outsider_email = format!("tor-nm-out-{}@example.com", Uuid::new_v4());
    let (outsider_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &outsider_email).await;

    let team_slug = format!("tor-nm-team-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (team_id, context_id) = insert_team_owned_context(&pool, &team_slug, "secret").await;
    add_team_member(&pool, team_id, owner_id).await;
    // Deliberately do NOT add the outsider as a member.

    let resource_id = home_resource_in_context(&pool, context_id, owner_id, "Secret note").await;

    assert!(
        !resource_visible(&pool, outsider_id, resource_id).await,
        "non-member must NOT see a resource in a team-owned context (no leak)"
    );
    assert!(
        !context_anchor_readable(&pool, outsider_id, context_id).await,
        "non-member must NOT read the team-owned context anchor (no leak)"
    );
}
