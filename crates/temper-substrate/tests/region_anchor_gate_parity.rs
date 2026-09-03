#![cfg(feature = "artifact-tests")]
//! **The region gate's two shapes must answer the same question.**
//!
//! Survey's Stage-1 admission gates candidate regions through `visible_region_anchors` — a SET
//! read over both anchor kinds — while every single-anchor read gates through
//! `anchor_readable_by_profile`, the scalar that dispatches per kind. Survey is the set read's
//! second consumer (wayfind's per-anchor filtering is the first), and its admission is recorded as
//! such; this file is the pin that the recorded second use cannot silently drift from the scalar.
//!
//! The two bodies are not one body and are not being asked to become one: the set read unions the
//! per-kind arm functions (`cogmap_visible_maps`, `contexts_readable_by`), the scalar dispatches
//! to the per-kind scalar predicates, and for contexts the scalar today DELEGATES to the set arm
//! (`context_readable_by_profile` is an EXISTS over `contexts_readable_by`) — structural agreement
//! that would be lost the day someone inlines it. For cogmaps the two are genuinely separate
//! bodies (a direct team-join+grant EXISTS against `cogmap_visible_maps`' UNION). Separation is
//! fine; disagreement is not, because a survey would then admit regions an anchored read refuses,
//! and the caller sees region hits for anchors the rest of the contract calls unreadable.
//!
//! So: one fixture corpus spanning every arm (personal, team-owned, team-shared by grant, team
//! cogmap, granted cogmap, and the denied side of each), and the assertion that the set read's
//! answer for an anchor EQUALS the scalar's — with the fixture asserting on its own data first, so
//! a corpus that cannot go red proves nothing.

use sqlx::{PgPool, Row};
use uuid::Uuid;

async fn create_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn create_team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn add_member(pool: &PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .unwrap();
}

async fn create_map_joined_to(pool: &PgPool, name: &str, team: Uuid) -> Uuid {
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("{name}-telos"))
    .bind(format!("temper://parity/{name}/telos"))
    .fetch_one(pool)
    .await
    .unwrap();
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();
    cogmap
}

async fn create_context(pool: &PgPool, owner_table: &str, owner_id: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), $1, $2, $3, $3) RETURNING id",
    )
    .bind(owner_table)
    .bind(owner_id)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// An explicit read grant on an anchor to `profile`.
async fn grant_read(pool: &PgPool, anchor_table: &str, anchor_id: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, \
            can_read, can_write, granted_by_profile_id) \
         VALUES ($1, $2, 'kb_profiles', $3, true, false, $3)",
    )
    .bind(anchor_table)
    .bind(anchor_id)
    .bind(profile)
    .execute(pool)
    .await
    .unwrap();
}

async fn scalar_readable(
    pool: &PgPool,
    profile: Uuid,
    anchor_table: &str,
    anchor_id: Uuid,
) -> bool {
    sqlx::query_scalar("SELECT anchor_readable_by_profile($1, $2, $3)")
        .bind(profile)
        .bind(anchor_table)
        .bind(anchor_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn set_visible(pool: &PgPool, profile: Uuid) -> Vec<(String, Uuid)> {
    sqlx::query("SELECT anchor_table, anchor_id FROM visible_region_anchors($1)")
        .bind(profile)
        .map(|row: sqlx::postgres::PgRow| {
            (
                row.get::<String, _>("anchor_table"),
                row.get::<Uuid, _>("anchor_id"),
            )
        })
        .fetch_all(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_region_gates_two_shapes_answer_identically_per_anchor(pool: PgPool) {
    let p = create_profile(&pool, "reader").await;
    let other = create_profile(&pool, "other").await;
    let team = create_team(&pool, "parity-team").await;
    let other_team = create_team(&pool, "elsewhere").await;
    add_member(&pool, team, p).await;

    // The corpus: every arm the set read has, plus the denied side of each.
    let personal_ctx = create_context(&pool, "kb_profiles", p, "personal").await; // personal arm
    let team_ctx = create_context(&pool, "kb_teams", team, "team-owned").await; // team-owned arm
    let granted_ctx = create_context(&pool, "kb_profiles", other, "granted").await; // explicit grant
    let denied_ctx = create_context(&pool, "kb_profiles", other, "denied").await; // nothing
    let team_map = create_map_joined_to(&pool, "team-map", team).await; // cogmap team arm
    let granted_map = create_map_joined_to(&pool, "granted-map", other_team).await; // cogmap grant arm
    let denied_map = create_map_joined_to(&pool, "denied-map", other_team).await; // nothing
    grant_read(&pool, "kb_contexts", granted_ctx, p).await;
    grant_read(&pool, "kb_cogmaps", granted_map, p).await;

    let corpus: Vec<(&str, Uuid, bool)> = vec![
        ("kb_contexts", personal_ctx, true),
        ("kb_contexts", team_ctx, true),
        ("kb_contexts", granted_ctx, true),
        ("kb_contexts", denied_ctx, false),
        ("kb_cogmaps", team_map, true),
        ("kb_cogmaps", granted_map, true),
        ("kb_cogmaps", denied_map, false),
    ];

    // The fixture asserts on its own data before the comparison: the scalar side must already
    // partition the corpus exactly as the fixture claims, or the corpus cannot go red.
    for (table, id, expected) in &corpus {
        assert_eq!(
            scalar_readable(&pool, p, table, *id).await,
            *expected,
            "fixture self-check: {table} anchor {id}"
        );
    }

    // THE PARITY. The set read's membership equals the scalar's answer, anchor by anchor —
    // both directions, so an extra set row and a missing one fail alike.
    let set = set_visible(&pool, p).await;
    for (table, id, expected) in &corpus {
        let in_set = set.iter().any(|(t, a)| t == table && a == id);
        assert_eq!(
            in_set, *expected,
            "the set read and the scalar disagree on {table} anchor {id}"
        );
        assert_eq!(
            scalar_readable(&pool, p, table, *id).await,
            *expected,
            "the scalar flipped against the fixture on {table} anchor {id}"
        );
    }
    // Counted over the corpus only: the seed births system anchors the corpus does not adjudicate,
    // and those are the scalar's business too (they agree — every corpus row was checked above).
    let corpus_visible = corpus.iter().filter(|(_, _, visible)| *visible).count();
    let in_corpus = set
        .iter()
        .filter(|(t, a)| corpus.iter().any(|(ct, ca, _)| ct == t && ca == a))
        .count();
    assert_eq!(
        in_corpus, corpus_visible,
        "the set read returns exactly the readable corpus anchors, no extras: {set:?}"
    );

    // THE LOCKSTEP. Revoke one grant and BOTH shapes must flip together — the property the
    // survey gate's second use rests on is that the two shapes move as one, in both directions.
    sqlx::query(
        "DELETE FROM kb_access_grants WHERE subject_table = 'kb_contexts' AND subject_id = $1",
    )
    .bind(granted_ctx)
    .execute(&pool)
    .await
    .unwrap();
    let set_after = set_visible(&pool, p).await;
    assert!(
        !set_after
            .iter()
            .any(|(t, a)| t == "kb_contexts" && *a == granted_ctx),
        "the revoked context must leave the set read"
    );
    assert!(
        !scalar_readable(&pool, p, "kb_contexts", granted_ctx).await,
        "and the scalar must flip with it"
    );
}
