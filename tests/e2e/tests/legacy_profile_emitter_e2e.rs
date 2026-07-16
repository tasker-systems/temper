#![cfg(feature = "test-db")]

//! End-to-end coverage for the legacy-profile emitter + default-context backfill
//! (`20260716000020_backfill_legacy_profile_emitters.sql`).
//!
//! Two approved, active human profiles in production (`gm-anirudh`, `lohjishan`) carry **zero**
//! `kb_entities` and **zero** contexts: they predate the canonical schema and were carried in by a
//! legacy import, so `profile_service::provision_profile_entities` — which creates the emitters and
//! the default context together — never ran for them. `writes::resolve_emitter` is a `fetch_one`
//! with no lazy creation, so their first write 500s. It is latent only because neither has ever
//! written.
//!
//! The distinction this file leans on: **creating a context fires no event, creating a resource
//! does.** So a legacy-shaped profile can make a context and still fail on the resource — which is
//! exactly why the failure stayed invisible, and why the assertion that matters is a real
//! `resources().create()` through the client, not a `resolve_emitter` probe. A SQL-only assertion
//! would pass against a schema that still 500s at the caller.
//!
//! Sibling: `sdk_emitter_entity_e2e.rs` covers the *partial* legacy shape (provisioned, missing one
//! surface). This file covers the *unprovisioned* shape its `EXISTS (<handle>@web)` guard could not
//! reach.

mod common;

use sqlx::PgPool;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_workflow::operations::Surface;
use temper_workflow::types::resource::ResourceCreateRequest;

/// The shipped backfill migration, executed verbatim. Reading the file rather than retyping its SQL
/// means this test cannot drift from what actually runs against production.
const BACKFILL_SQL: &str =
    include_str!("../../../migrations/20260716000020_backfill_legacy_profile_emitters.sql");

async fn run_backfill(pool: &PgPool) {
    sqlx::raw_sql(BACKFILL_SQL)
        .execute(pool)
        .await
        .expect("run backfill migration");
}

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

/// The emitter names `Surface::ALL` implies for `handle`, sorted. Derived from the enum rather than
/// hand-listed, so a surface added without a backfill path fails here.
fn expected_emitters(handle: &str) -> Vec<String> {
    let mut names: Vec<String> = Surface::ALL
        .iter()
        .map(|s| format!("{handle}@{}", s.marker()))
        .collect();
    names.sort();
    names
}

async fn context_slugs_for(pool: &PgPool, profile_id: uuid::Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT slug FROM kb_contexts \
         WHERE owner_table = 'kb_profiles' AND owner_id = $1 ORDER BY slug",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await
    .expect("query context slugs")
}

async fn handle_of(pool: &PgPool, profile_id: uuid::Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT handle FROM kb_profiles WHERE id = $1")
        .bind(profile_id)
        .fetch_one(pool)
        .await
        .expect("profile handle")
}

/// Strip an auto-provisioned profile back to the shape production actually shows for `gm-anirudh`
/// and `lohjishan`: zero entities, zero contexts.
///
/// Reached by provisioning through the real JIT path and then removing the rows, rather than by
/// hand-inserting a bare `kb_profiles` row. Both produce the same end state, but this one asserts
/// its own precondition — if provisioning ever stops creating these, the delete counts change and
/// this helper's assertions fail rather than silently testing nothing.
async fn strip_to_legacy_shape(pool: &PgPool, profile_id: uuid::Uuid) {
    let entities = sqlx::query("DELETE FROM kb_entities WHERE profile_id = $1")
        .bind(profile_id)
        .execute(pool)
        .await
        .expect("strip emitter entities")
        .rows_affected();
    assert_eq!(
        entities,
        Surface::ALL.len() as u64,
        "precondition: provisioning should have created one emitter per surface to strip",
    );

    let contexts =
        sqlx::query("DELETE FROM kb_contexts WHERE owner_table = 'kb_profiles' AND owner_id = $1")
            .bind(profile_id)
            .execute(pool)
            .await
            .expect("strip contexts")
            .rows_affected();
    assert_eq!(
        contexts, 1,
        "precondition: provisioning should have created exactly the default context to strip",
    );
}

fn create_request(context_id: ContextId, title: &str) -> ResourceCreateRequest {
    ResourceCreateRequest {
        kb_context_id: *context_id,
        doc_type: "research".to_string(),
        origin_uri: format!("test://e2e/legacy-profile/{title}"),
        title: title.to_string(),
        act: Default::default(),
    }
}

/// **The headline test.** A legacy-shaped profile's resource create fails, and the backfill fixes
/// it — asserted at the production caller's level, through the real client and the real HTTP
/// surface, because that is where the 500 lands.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn legacy_profile_write_fails_then_succeeds_after_backfill(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("auto-provision profile on first authenticated request");

    strip_to_legacy_shape(&pool, profile.id).await;

    // A context needs no emitter — creating one fires no event. This is precisely why the bug is
    // invisible until a *resource* write: the profile looks workable right up to the failure.
    let context = app
        .client
        .contexts()
        .create("legacy-profile-ctx", None)
        .await
        .expect("context create needs no emitter, so it must succeed even in the legacy shape");

    // The bug, reproduced where users meet it.
    let err = app
        .client
        .resources()
        .create(&create_request(context.id, "before-backfill"))
        .await
        .expect_err(
            "precondition: a legacy profile's first resource write must fail — this is the 500 \
             the backfill exists to prevent",
        );
    // Pin the CAUSE, not merely the failure. Asserting "it errored" would pass for a missing
    // context, a bad doc_type, or an auth reject — none of which this migration fixes. The
    // observed error is a real HTTP 500 carrying `resolve_emitter`'s own context string:
    //   Server { status: 500, message: "Internal error: no emitter entity <handle>@cli for the
    //   resolved profile" }
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("500") && rendered.contains("no emitter entity"),
        "the write must 500 on the missing emitter specifically — otherwise this test would pass \
         for the wrong reason. Got: {rendered}",
    );

    run_backfill(&pool).await;

    // The fix, proved at the same level.
    let created = app
        .client
        .resources()
        .create(&create_request(context.id, "after-backfill"))
        .await
        .expect("a legacy profile's resource write must succeed after the backfill");
    assert_eq!(created.title, "after-backfill");
    assert!(created.is_active);

    let handle = handle_of(&pool, profile.id).await;
    assert_eq!(
        emitters_for(&pool, &handle).await,
        expected_emitters(&handle),
        "the backfill should bring the legacy profile up to the full surface set",
    );

    // NO default context here, and that is the design, not a miss. This profile made a context of
    // its own before the backfill landed, so the context guard ("no contexts at all") correctly
    // declines to second-guess it. The two guards are independent precisely so this case still gets
    // its emitters — which is what the 500 actually needed. The dormant shape that *does* receive a
    // default is covered by `dormant_legacy_profile_receives_its_default_context`.
    assert_eq!(
        context_slugs_for(&pool, profile.id).await,
        vec!["legacy-profile-ctx".to_string()],
        "a profile that made its own context before the backfill keeps exactly that context",
    );
}

/// The shape production actually carries: `gm-anirudh` and `lohjishan` are **dormant** — zero
/// entities, zero contexts, zero resources — so the backfill reaches them before they ever write.
/// This is the faithful end-to-end for those two rows: heal, then write into the default context
/// the migration restored, through the real client.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn dormant_legacy_profile_receives_its_default_context(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = app.client.profile().get().await.expect("provision profile");

    strip_to_legacy_shape(&pool, profile.id).await;
    run_backfill(&pool).await;

    assert_eq!(
        context_slugs_for(&pool, profile.id).await,
        vec!["default".to_string()],
        "a dormant legacy profile must get back the default context provisioning never created",
    );

    // The default context is not merely a row — it is usable by the caller that needed it.
    let default_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_contexts \
         WHERE owner_table = 'kb_profiles' AND owner_id = $1 AND slug = 'default'",
    )
    .bind(profile.id)
    .fetch_one(&pool)
    .await
    .expect("resolve the restored default context");

    let created = app
        .client
        .resources()
        .create(&create_request(ContextId::from(default_id), "into-default"))
        .await
        .expect("a healed dormant profile must be able to write into its restored default context");
    assert_eq!(created.title, "into-default");
}

/// Every surface resolves through the exact `fetch_one` a write performs. Counting rows would pass
/// even if a name were spelled wrong.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn backfill_provisions_every_surface_emitter_and_is_idempotent(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = app.client.profile().get().await.expect("provision profile");
    let handle = handle_of(&pool, profile.id).await;

    strip_to_legacy_shape(&pool, profile.id).await;
    for surface in Surface::ALL {
        assert!(
            temper_substrate::writes::resolve_emitter(
                &pool,
                ProfileId::from(profile.id),
                surface.marker(),
            )
            .await
            .is_err(),
            "precondition: no emitter resolves in the legacy shape ({})",
            surface.marker(),
        );
    }

    run_backfill(&pool).await;

    for surface in Surface::ALL {
        temper_substrate::writes::resolve_emitter(
            &pool,
            ProfileId::from(profile.id),
            surface.marker(),
        )
        .await
        .unwrap_or_else(|e| {
            panic!(
                "resolve_emitter({}) must succeed after the backfill: {e}",
                surface.marker()
            )
        });
    }

    // Idempotent on the strength of `ON CONFLICT DO NOTHING` against the unique index added by
    // 20260709000040 — not, as in the sdk backfill, on a `NOT EXISTS` guard written when no such
    // index existed. A second run must insert nothing rather than error.
    run_backfill(&pool).await;
    assert_eq!(
        emitters_for(&pool, &handle).await,
        expected_emitters(&handle),
        "a second run must insert nothing",
    );
    assert_eq!(
        context_slugs_for(&pool, profile.id).await,
        vec!["default".to_string()],
        "a second run must not duplicate the default context",
    );
}

/// `system` emits as the bare entity `system` and never resolves through `resolve_emitter`. The
/// guard excludes it because it **has an entity** — not on `handle <> 'system'`, which
/// 20260709000030 rejects as dishonest.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn backfill_skips_the_system_profile_on_a_structural_guard(pool: PgPool) {
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
        "the backfill must not manufacture unused per-surface emitters for system",
    );
}

/// A CONNECTION-shaped profile must not be swept in. `connection_service` mints its emitter inline
/// as `<handle>@webhook` (connection_service.rs:140) instead of calling
/// `provision_profile_entities`, and a connection emits over that one surface — it can never use
/// web/cli/mcp/sdk. When the connection is owned by a team its home context belongs to the TEAM, so
/// the profile itself owns no context and would also collect a spurious `default`.
///
/// This is a REGRESSION TEST for a real defect in an earlier draft of this migration, whose guard
/// keyed on the `system` shape (`e.name = p.handle`) and therefore did not exclude connection
/// profiles. Production carries no connections today, so it would have shipped silent. The
/// "has no entities at all" guard excludes them because they HAVE an entity — no shape is
/// enumerated, so the next principal kind that mints its own emitter is excluded for free.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_profile_that_mints_its_own_emitter_is_not_swept_in(pool: PgPool) {
    let handle = "connection-github-acme";
    let profile_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(&pool)
    .await
    .expect("seed a connection-shaped profile");

    // Its own emitter, and no context of its own — the team-owned-connection shape.
    sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
        .bind(profile_id)
        .bind(format!("{handle}@webhook"))
        .execute(&pool)
        .await
        .expect("seed the webhook emitter");

    run_backfill(&pool).await;

    assert_eq!(
        emitters_for(&pool, handle).await,
        vec![format!("{handle}@webhook")],
        "a profile that mints its own emitter must not collect four surface emitters it can never \
         use — the same error the system exclusion exists to prevent",
    );
    assert!(
        context_slugs_for(&pool, profile_id).await.is_empty(),
        "nor a default context: a team-owned connection homes on the team, not on this profile",
    );
}

/// The context half is guarded on having **no contexts at all**, not on lacking a `default`.
///
/// Production's `j-cole-taylor` holds six contexts and none of them is slugged `default`. A naive
/// "provision a default for every profile lacking one" guard would silently resurrect a context
/// that account never asked for. Absence of a `default` is not evidence of a missing provisioning
/// run; absence of *every* context is.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn backfill_does_not_resurrect_a_default_for_a_profile_with_its_own_contexts(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = app.client.profile().get().await.expect("provision profile");

    // Delete only the default context, leaving one the user made: the production shape.
    app.client
        .contexts()
        .create("a-context-of-my-own", None)
        .await
        .expect("context create");
    sqlx::query(
        "DELETE FROM kb_contexts \
         WHERE owner_table = 'kb_profiles' AND owner_id = $1 AND slug = 'default'",
    )
    .bind(profile.id)
    .execute(&pool)
    .await
    .expect("drop the default context");

    run_backfill(&pool).await;

    assert_eq!(
        context_slugs_for(&pool, profile.id).await,
        vec!["a-context-of-my-own".to_string()],
        "the backfill must not resurrect a default context for a profile that has its own",
    );
}
