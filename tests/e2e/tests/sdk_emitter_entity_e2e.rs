#![cfg(feature = "test-db")]

//! End-to-end coverage for the `sdk` per-surface emitter entity (task P1 of the temper-rb goal).
//!
//! Every `kb_events` row carries a NOT NULL `emitter_entity_id`, resolved by
//! `writes::resolve_emitter` as the natural key `<handle>@<surface>` — with a `fetch_one`, so a
//! missing entity is a hard error rather than a lazy create. Two paths must therefore produce the
//! `sdk` emitter:
//!
//! 1. **New profiles** — `profile_service::provision_profile_entities`, driven off `Surface::ALL`.
//! 2. **Existing profiles** — the additive backfill migration, executed here verbatim.
//!
//! The wire-level assertion (an `X-Temper-Surface: sdk` request attributing to `<handle>@sdk`)
//! landed in P2 and lives in `surface_attribution_e2e.rs`. What this file owes is that the
//! entity resolves at all.

mod common;

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_workflow::operations::Surface;

/// The shipped backfill migration, executed verbatim. Reading the file rather than retyping its
/// SQL means this test cannot drift from what actually runs against production.
const BACKFILL_SQL: &str =
    include_str!("../../../migrations/20260709000030_backfill_sdk_emitter_entities.sql");

/// Every emitter entity name belonging to `handle`, sorted.
async fn emitters_for(pool: &PgPool, handle: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT e.name FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
         WHERE p.handle = $1 ORDER BY e.name",
    )
    .bind(handle)
    .fetch_all(pool)
    .await
    .expect("query emitter entities")
}

/// The emitter names `Surface::ALL` implies for `handle`, sorted. Derived from the enum rather
/// than hand-listed, so a surface added without a provisioning path fails here.
fn expected_emitters(handle: &str) -> Vec<String> {
    let mut names: Vec<String> = Surface::ALL
        .iter()
        .map(|s| format!("{handle}@{}", s.marker()))
        .collect();
    names.sort();
    names
}

async fn handle_of(pool: &PgPool, profile_id: uuid::Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT handle FROM kb_profiles WHERE id = $1")
        .bind(profile_id)
        .fetch_one(pool)
        .await
        .expect("profile handle")
}

/// Seed a profile as it looked *before* this migration: provisioned for the three original
/// surfaces, with no `@sdk` emitter.
async fn seed_legacy_profile(pool: &PgPool, handle: &str) -> uuid::Uuid {
    let profile_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("seed legacy profile");

    for marker in ["web", "cli", "mcp"] {
        sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
            .bind(profile_id)
            .bind(format!("{handle}@{marker}"))
            .execute(pool)
            .await
            .expect("seed legacy emitter");
    }
    profile_id
}

async fn run_backfill(pool: &PgPool) {
    sqlx::raw_sql(BACKFILL_SQL)
        .execute(pool)
        .await
        .expect("run backfill migration");
}

/// A profile auto-provisioned on its first authenticated request carries one emitter per surface —
/// four of them now — and each resolves through the real `fetch_one` the write path uses.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn fresh_profile_is_provisioned_with_every_surface_emitter(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("auto-provision profile on first authenticated request");

    let handle = handle_of(&pool, profile.id).await;

    assert_eq!(
        emitters_for(&pool, &handle).await,
        expected_emitters(&handle),
        "a fresh profile should carry exactly one emitter per Surface::ALL variant",
    );

    // The assertion that matters: the exact `fetch_one` a write performs. Counting rows would
    // pass even if the name were spelled wrong.
    for surface in Surface::ALL {
        temper_substrate::writes::resolve_emitter(
            &pool,
            ProfileId::from(profile.id),
            surface.marker(),
        )
        .await
        .unwrap_or_else(|e| {
            panic!(
                "resolve_emitter({}) failed for a fresh profile: {e}",
                surface.marker()
            )
        });
    }
}

/// A profile that predates the `sdk` surface gains its `@sdk` emitter from the backfill, and the
/// backfill is idempotent — it has no unique constraint to lean on, only its `NOT EXISTS` guard.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn backfill_provisions_sdk_for_a_legacy_profile_and_is_idempotent(pool: PgPool) {
    let handle = "legacy-user";
    let profile_id = seed_legacy_profile(&pool, handle).await;

    assert_eq!(
        emitters_for(&pool, handle).await,
        vec![
            format!("{handle}@cli"),
            format!("{handle}@mcp"),
            format!("{handle}@web"),
        ],
        "precondition: the legacy profile has no @sdk emitter",
    );
    assert!(
        temper_substrate::writes::resolve_emitter(&pool, ProfileId::from(profile_id), "sdk")
            .await
            .is_err(),
        "precondition: resolving the sdk emitter must fail before the backfill — this is the 500 \
         that migrate-ahead-of-deploy exists to prevent",
    );

    run_backfill(&pool).await;

    assert_eq!(
        emitters_for(&pool, handle).await,
        expected_emitters(handle),
        "the backfill should bring the legacy profile up to the full surface set",
    );
    temper_substrate::writes::resolve_emitter(&pool, ProfileId::from(profile_id), "sdk")
        .await
        .expect("sdk emitter resolves after the backfill");

    // `kb_entities` has no unique constraint on (profile_id, name), so a second run guarded only
    // by NOT EXISTS is the sole thing standing between us and a duplicate emitter — which would
    // make `resolve_emitter`'s `fetch_one` ambiguous.
    run_backfill(&pool).await;
    assert_eq!(
        emitters_for(&pool, handle).await,
        expected_emitters(handle),
        "the backfill must be idempotent: a second run inserts nothing",
    );
}

/// The `system` profile emits as the bare entity `system`, never through `resolve_emitter`. The
/// backfill's `EXISTS (<handle>@web)` guard must leave it alone rather than manufacture an
/// unused `system@sdk`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn backfill_skips_the_unprovisioned_system_profile(pool: PgPool) {
    let before = emitters_for(&pool, "system").await;
    assert_eq!(
        before,
        vec!["system".to_string()],
        "precondition: the canonical seed gives system a bare emitter, not a per-surface set",
    );

    run_backfill(&pool).await;

    assert_eq!(
        emitters_for(&pool, "system").await,
        before,
        "the backfill should not touch a profile that never had per-surface emitters",
    );
}
