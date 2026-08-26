#![cfg(feature = "artifact-tests")]
//! The witness a hard delete could not have passed: `create → rename → retire → replay`, plus
//! its mirror, `create → rename → retire → restore → replay`.
//!
//! `20260825000030_context_retirement.sql`'s own rationale is exactly this: `kb_contexts` is a
//! replay INPUT table restored verbatim (`INPUT_TABLES` in `replay.rs`), and
//! `_project_context_renamed` `RAISE`s `context_rename: context % not found` on a missing row
//! (`20260731000040_context_rename_fns.sql:48`), which `replay::replay`'s
//! `EventKind::ContextRenamed` arm calls **unguarded** — no existence check, no catch. Under the
//! hard delete this file's design supersedes, a context that was ever renamed and then deleted
//! made replay abort outright: the row the projector needs
//! is gone, `_project_context_renamed` raises, `replay::replay` propagates the error, and the
//! whole ledger walk fails — not just the one event. **Retirement leaves the row in place.** It
//! is still there — under a different, mangled slug, with `is_active = false` — for
//! `_project_context_renamed` to find and re-apply its `(to_name, to_slug)` UPDATE onto, exactly
//! as it does for a context that was never touched again after the rename. `is_active` alone would
//! ride in with `kb_contexts`'s own verbatim restore, the way `kb_teams.is_active` does for team
//! soft-delete — the MANGLED SLUG does not. `replay::replay` walks the ledger `ORDER BY e.id`,
//! and `kb_events.id` defaults to `uuid_generate_v7()`, so the EARLIER `context_renamed`
//! re-applies on top of the restored row and `_project_context_renamed` drives `slug` back to its
//! pre-retirement value. `_project_context_retired`, replaying after it, is what puts the mangle
//! back — which is what the closing assertion of `a_retired_context_replays` catches.
//!
//! Modeled directly on `context_rename_replay.rs` — same fixture shape
//! (`bootseed::seed_system` + a team-owned context + a maintainer emitter), same use of
//! `writes::rename_context_with` for the evented half, and the same snapshot → reset → replay
//! sequencing.
//!
//! **This test deliberately does NOT call `common::reset_schema` up front**, for the identical
//! reason `context_rename_replay.rs` records: `context_renamed` is registered by its own
//! migration and is absent from `tests/fixtures/seeds/system.yaml`'s `event_types` (as
//! `context_reassigned` and `citation_audited` are), so an initial reset would TRUNCATE it out of
//! `kb_event_types` and `_event_append` would refuse the rename's fire. The reset belongs only
//! between snapshot and replay, where `replay()` restores the registry verbatim.
//!
//! **Retirement is EVENTED, and this test is why.** It was originally built un-evented, matching
//! `kb_teams`' soft-delete — and this witness failed, which is how the defect was found:
//!
//! ```text
//! expected: ("Annual Planning", "annual-planning-retired", false)
//! got:      ("Annual Planning", "annual-planning",         false)
//! ```
//!
//! `is_active = false` survived, because no event touched it. The MANGLED SLUG did not.
//! `_project_context_renamed` sets `slug` from its payload, so replaying the earlier rename drove
//! the slug back to its pre-retirement value — and with `UNIQUE (owner_table, owner_id, slug)`, a
//! new context created under the freed name would then collide and abort the replay outright: the
//! same class of failure as the hard delete this design replaced, by a different route.
//!
//! `kb_teams` gets away with an un-evented soft-delete because it does not touch an
//! identity-bearing column. Retirement moves the slug, and identity-bearing columns are exactly
//! what `rename` and `reassign` are evented for. Hence `20260825000040_context_retire_fns.sql`.
//!
//! So the retirement here fires through `writes::retire_context_with`, under real authority, the
//! same standard `context_rename_replay.rs` holds its rename to. A raw `UPDATE` would pass this
//! test while proving nothing.

mod common;

use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId};
use temper_substrate::{replay, scenario::bootseed, writes};
use uuid::Uuid;

const OLD_NAME: &str = "Quarterly Planning";
const OLD_SLUG: &str = "quarterly-planning";
const NEW_NAME: &str = "Annual Planning";
const NEW_SLUG: &str = "annual-planning";
const RETIRED_SLUG: &str = "annual-planning-retired";

/// An entity for `profile` — `context_rename` authorizes `kb_entities.profile_id`, not the
/// entity. Copied verbatim from `context_rename_replay.rs`.
async fn insert_entity(pool: &sqlx::PgPool, profile: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) \
         VALUES ($1, $2, '{}'::jsonb) RETURNING id",
    )
    .bind(profile)
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("insert entity")
}

/// Direct membership at an administering role — `common::add_team_member` inserts `'member'`,
/// which `context_rename`'s gate (`role IN ('owner','maintainer')`) would refuse. Copied verbatim
/// from `context_rename_replay.rs`.
async fn add_maintainer(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'maintainer')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .expect("add maintainer");
}

/// `(name, slug, is_active)` for a context — the row shape a retirement plus a prior rename both
/// touch, read together so one query can assert the whole post-replay state.
async fn context_state(pool: &sqlx::PgPool, context: Uuid) -> (String, String, bool) {
    sqlx::query_as("SELECT name, slug, is_active FROM kb_contexts WHERE id = $1")
        .bind(context)
        .fetch_one(pool)
        .await
        .expect("read context state")
}

/// Retire through the PRODUCTION write path, under real authority — the same standard
/// `context_rename_replay.rs` holds its rename to ("The rename lands through the production write
/// path, under real authority"). A raw `UPDATE` here would pass while proving nothing: the whole
/// point of this witness is that the retirement reaches the ledger.
async fn retire_context(pool: &sqlx::PgPool, context: Uuid, emitter: Uuid) {
    writes::retire_context_with(
        pool,
        ContextId::from(context),
        NEW_SLUG,
        RETIRED_SLUG,
        EntityId::from(emitter),
        EventContext::default(),
    )
    .await
    .expect("retire under maintainer authority");
}

/// Restore through the PRODUCTION write path, mirroring `retire_context`. The slug arguments match
/// what `context_service::restore` passes: `from_slug` is the mangled address the row currently
/// holds, `to_slug` the address re-derived from the untouched name by
/// `next_unique_context_slug` — which here is `NEW_SLUG`, since the only row in this owner's slug
/// space is the retired context itself, and that lookup is `is_active`-blind.
async fn restore_context(pool: &sqlx::PgPool, context: Uuid, emitter: Uuid) {
    writes::restore_context_with(
        pool,
        ContextId::from(context),
        RETIRED_SLUG,
        NEW_SLUG,
        EntityId::from(emitter),
        EventContext::default(),
    )
    .await
    .expect("restore under maintainer authority");
}

/// `create → rename → retire`, then a full ledger replay. Under the hard delete this design
/// supersedes, this is exactly where replay aborted: `_project_context_renamed` would `RAISE`
/// `context_rename: context % not found` against a row a hard delete had removed, and
/// `replay::replay` would propagate that error instead of completing. A retired context is still
/// a row, so `_project_context_renamed` finds it — and re-applies its `(to_name, to_slug)`
/// UPDATE, taking the slug straight back off the mangle. `is_active = false` does ride in with
/// `kb_contexts`'s own verbatim input-table restore; the mangled slug is put back by
/// `_project_context_retired` running after the rename, and the closing assertion is what would
/// catch its absence.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_retired_context_replays(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();

    let team = common::create_team(&pool, "planners").await;
    let profile = common::create_profile(&pool, "maintainer@example.com").await;
    let emitter = insert_entity(&pool, profile, "maintainer#1").await;
    add_maintainer(&pool, team, profile).await;
    let context = common::insert_context(&pool, "kb_teams", team, OLD_SLUG, OLD_NAME)
        .await
        .expect("insert team-owned context");

    // The rename lands through the production write path — the evented mutation whose projector
    // is the one that would explode under a hard delete.
    writes::rename_context_with(
        &pool,
        ContextId::from(context),
        (OLD_NAME, OLD_SLUG),
        (NEW_NAME, NEW_SLUG),
        EntityId::from(emitter),
        EventContext::default(),
    )
    .await
    .expect("rename under maintainer authority");
    assert_eq!(
        context_state(&pool, context).await,
        (NEW_NAME.to_string(), NEW_SLUG.to_string(), true),
        "the rename applied and the context is still active"
    );

    // NON-VACUITY: the ledger must actually carry the rename event, or the replay walk would
    // never reach the `EventKind::ContextRenamed` arm that this test is about, and `replay()`
    // returning `Ok` would prove nothing about it.
    let renamed_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'context_renamed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(renamed_events, 1, "exactly one context_renamed event fired");

    // Retire the SAME context the rename already touched, through the evented write path. Both
    // events now sit in the ledger, and replay must re-apply them in order: rename sets the slug
    // to NEW_SLUG, then retire moves it to RETIRED_SLUG. Ledger order is what makes the mangle
    // survive.
    retire_context(&pool, context, emitter).await;
    assert_eq!(
        context_state(&pool, context).await,
        (NEW_NAME.to_string(), RETIRED_SLUG.to_string(), false),
        "retirement flips is_active and mangles the slug, and leaves the row (and its renamed \
         name) in place"
    );

    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;

    // THE CLAUSE. Under the hard delete this design supersedes, the context row would be gone by
    // this point and this call would fail with `context_rename: context % not found` propagated
    // straight out of `_project_context_renamed`. It does not, because retirement never deleted
    // the row.
    replay::replay(&pool, &snap)
        .await
        .expect("replay must succeed: retirement preserves the row _project_context_renamed needs");

    // THE RETIRE PROJECTOR'S WITNESS — do not delete this as a redundant re-apply check.
    // `kb_contexts` is a replay INPUT table, so the row does come back carrying (name, slug,
    // is_active) as they stood at the snapshot, and `is_active = false` genuinely rides in that
    // way. The slug does not stay put: the walk is `ORDER BY e.id` over `uuid_generate_v7()` ids,
    // so the EARLIER `context_renamed` re-applies first and `_project_context_renamed` sets `slug`
    // from its own payload. Stepping the two projector bodies over a restored row gives:
    //
    //   start                    annual-planning-retired   f
    //   after rename projector   annual-planning           f   <- clobbered
    //   after retire projector   annual-planning-retired   f   <- restored
    //
    // So gutting `replay`'s `EventKind::ContextRetired` arm fails this assertion on the slug, and
    // no other file under `crates/` calls `writes::retire_context_with` outside
    // `context_service::retire` itself, so nothing else reaches that arm. `kb_contexts` is absent
    // from `PROJECTION_DUMPS`, so replay's generic round-trip diff would not notice either.
    assert_eq!(
        context_state(&pool, context).await,
        (NEW_NAME.to_string(), RETIRED_SLUG.to_string(), false),
        "the retire projector re-applied the mangled slug on top of the replayed rename, and the \
         retirement flag survived the verbatim input-table restore"
    );
}

/// `create → rename → retire → restore`, then a full ledger replay — the mirror witness, for
/// `replay`'s own `EventKind::ContextRestored` arm. That arm had no test: no other file under
/// `crates/` calls `writes::restore_context_with` outside `context_service::restore` itself, and
/// `kb_contexts` is a replay INPUT table absent from `PROJECTION_DUMPS`, so replay's generic
/// round-trip diff cannot see a wrong context projection. Only an assertion here can.
///
/// The rename is kept for parity with `a_retired_context_replays`, and to keep
/// `_project_context_renamed` in this walk against a row that was retired and restored under it.
/// It is not what makes this one bite — see the closing assertion.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_restored_context_replays(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();

    let team = common::create_team(&pool, "planners").await;
    let profile = common::create_profile(&pool, "maintainer@example.com").await;
    let emitter = insert_entity(&pool, profile, "maintainer#1").await;
    add_maintainer(&pool, team, profile).await;
    let context = common::insert_context(&pool, "kb_teams", team, OLD_SLUG, OLD_NAME)
        .await
        .expect("insert team-owned context");

    writes::rename_context_with(
        &pool,
        ContextId::from(context),
        (OLD_NAME, OLD_SLUG),
        (NEW_NAME, NEW_SLUG),
        EntityId::from(emitter),
        EventContext::default(),
    )
    .await
    .expect("rename under maintainer authority");

    retire_context(&pool, context, emitter).await;
    restore_context(&pool, context, emitter).await;
    assert_eq!(
        context_state(&pool, context).await,
        (NEW_NAME.to_string(), NEW_SLUG.to_string(), true),
        "restore flips is_active back and re-derives the slug off the untouched name"
    );

    // NON-VACUITY: without a `context_restored` row in the ledger the walk never reaches the
    // `EventKind::ContextRestored` arm, and `replay()` returning `Ok` would prove nothing about it.
    let restored_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'context_restored'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        restored_events, 1,
        "exactly one context_restored event fired"
    );

    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;

    replay::replay(&pool, &snap)
        .await
        .expect("replay must succeed across rename → retire → restore");

    // THE RESTORE PROJECTOR'S WITNESS. The input-table restore brings the row back at the restored
    // state, and the replayed rename is a no-op on it (same name, same slug) — but the replayed
    // `context_retired` that follows is NOT: `_project_context_retired` sets `is_active = false`
    // and drives `slug` to its own `to_slug`. Stepping the projector bodies over a restored row:
    //
    //   start (input restore)     annual-planning           t
    //   after rename projector    annual-planning           t   <- no-op here
    //   after retire projector    annual-planning-retired   f   <- clobbered
    //   after restore projector   annual-planning           t   <- restored
    //
    // Gut `replay`'s `EventKind::ContextRestored` arm and the walk stops at the third line: this
    // assertion then fails on BOTH the slug and the flag.
    assert_eq!(
        context_state(&pool, context).await,
        (NEW_NAME.to_string(), NEW_SLUG.to_string(), true),
        "the restore projector undid the replayed retirement: active again, at the re-derived \
         slug"
    );
}
