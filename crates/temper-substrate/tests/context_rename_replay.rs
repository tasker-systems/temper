#![cfg(feature = "artifact-tests")]
//! `replayed-history-is-not-re-adjudicated` — the register clause whose entire content is that
//! `_project_context_renamed` never authorizes, so a rename that was valid when it happened stays
//! valid when it is re-applied.
//!
//! Spec: `internal/superpowers/specs/2026-07-30-context-rename-design.md` §"Shape" —
//! *"A pure re-apply that **never authorizes**, so replayed history is not re-adjudicated against
//! present-day membership."* Migration `20260730000020_context_rename_fns.sql` states the same
//! split: the RBAC gate is an invariant of `context_rename` (the mutation half), and the projector
//! half stays a pure `UPDATE`.
//!
//! Plan Part 5 §"`replayed-history-is-not-re-adjudicated` — the one clause with no evidence at all"
//! is why this file exists: every other clause on the branch ends with a biting test, and this one
//! would otherwise end with an argument about the source.
//!
//! No ONNX, no scenario machinery — direct fixtures on the `MIGRATOR`-provisioned ephemeral DB, the
//! shape `write_path_mutations::replay_reproduces_mutation_projections` uses.
//!
//! **This test deliberately does NOT call `common::reset_schema` up front.** `context_renamed` is
//! registered by its migration and is absent from `tests/fixtures/seeds/system.yaml`'s
//! `event_types` (as `context_reassigned` and `citation_audited` are), so an initial reset would
//! TRUNCATE it out of `kb_event_types` and `_event_append` would refuse the fire. The reset belongs
//! only between snapshot and replay, where `replay()` restores the registry verbatim.

mod common;

use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId};
use temper_substrate::{replay, scenario::bootseed, writes};
use uuid::Uuid;

const OLD_NAME: &str = "Quarterly Planning";
const OLD_SLUG: &str = "quarterly-planning";
const NEW_NAME: &str = "Annual Planning";
const NEW_SLUG: &str = "annual-planning";

/// An entity for `profile` — `context_rename` authorizes `kb_entities.profile_id`, not the entity.
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
/// which `context_rename`'s gate (`role IN ('owner','maintainer')`) would refuse.
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

async fn context_identity(pool: &sqlx::PgPool, context: Uuid) -> (String, String) {
    sqlx::query_as("SELECT name, slug FROM kb_contexts WHERE id = $1")
        .bind(context)
        .fetch_one(pool)
        .await
        .expect("read context identity")
}

/// The mutation half, called directly so its refusal SQLSTATE is observable.
async fn call_context_rename(
    pool: &sqlx::PgPool,
    context: Uuid,
    emitter: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    let payload = serde_json::json!({
        "context_id": context,
        "from_name": NEW_NAME,
        "from_slug": NEW_SLUG,
        "to_name": "Rejected Rename",
        "to_slug": "rejected-rename",
    });
    sqlx::query_scalar("SELECT context_rename($1, $2)")
        .bind(payload)
        .bind(emitter)
        .fetch_one(pool)
        .await
}

/// A `context_renamed` event whose emitter has since lost the authority that admitted it still
/// replays. **The load-bearing assertion is that `replay::replay` returns `Ok`** — it is the whole
/// content of `replayed-history-is-not-re-adjudicated`, and it fails the moment
/// `_project_context_renamed` grows an authorization check (the projector would raise `42501`,
/// `replay()` would propagate it, and the `expect` below would panic).
///
/// The revocation happens BEFORE the snapshot on purpose: `kb_team_members` is a replay INPUT table
/// (`replay.rs:89-114`), restored verbatim, so a row deleted before the snapshot is absent from it
/// and therefore absent after the restore. At replay time the actor genuinely holds no authority.
/// `revoking_the_admitting_membership_makes_the_mutation_half_refuse` below pins that the
/// revocation is real rather than assumed.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn renamed_context_replays_after_its_emitter_lost_authority(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();

    let team = common::create_team(&pool, "planners").await;
    let profile = common::create_profile(&pool, "maintainer@example.com").await;
    let emitter = insert_entity(&pool, profile, "maintainer#1").await;
    add_maintainer(&pool, team, profile).await;
    let context = common::insert_context(&pool, "kb_teams", team, OLD_SLUG, OLD_NAME)
        .await
        .expect("insert team-owned context");

    // The rename lands through the production write path, under real authority.
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
        context_identity(&pool, context).await,
        (NEW_NAME.to_string(), NEW_SLUG.to_string()),
        "the authorizing half applied the rename"
    );

    // NON-VACUITY: the ledger must actually carry the event, or the replay walk would never reach
    // the `EventKind::ContextRenamed` arm and `replay()` returning Ok would prove nothing.
    let renamed_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'context_renamed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(renamed_events, 1, "exactly one context_renamed event fired");

    // Revoke the authority that admitted the rename. Nothing else changes.
    let revoked = sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(team)
        .bind(profile)
        .execute(&pool)
        .await
        .expect("revoke membership")
        .rows_affected();
    assert_eq!(revoked, 1, "the admitting membership row was deleted");

    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;

    // THE CLAUSE. Present-day authority is gone; the history still replays.
    replay::replay(&pool, &snap)
        .await
        .expect("replay must not re-adjudicate a rename against present-day authority");

    // Re-apply check, NOT the clause's evidence. `kb_contexts` is a replay INPUT table restored
    // verbatim, so it comes back ALREADY carrying the renamed values — this assertion would pass
    // even if the projector had done nothing at all. It is here to catch a projector that runs and
    // writes the WRONG pair (e.g. reading `from_*`), never to demonstrate that it ran.
    assert_eq!(
        context_identity(&pool, context).await,
        (NEW_NAME.to_string(), NEW_SLUG.to_string()),
        "the projector's re-apply is idempotent on the restored row"
    );
}

/// The revocation above is real, not assumed: the SAME actor, on the SAME context, is refused by
/// the mutation half with SQLSTATE `42501`. Without this, "replay succeeded under revoked
/// authority" could be satisfied by an actor who never lost anything.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn revoking_the_admitting_membership_makes_the_mutation_half_refuse(pool: sqlx::PgPool) {
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

    sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(team)
        .bind(profile)
        .execute(&pool)
        .await
        .expect("revoke membership");

    let err = call_context_rename(&pool, context, emitter)
        .await
        .expect_err("the mutation half must refuse a demoted actor");
    match err {
        sqlx::Error::Database(db) => assert_eq!(
            db.code().as_deref(),
            Some("42501"),
            "insufficient_privilege, not some other failure"
        ),
        other => panic!("expected a database refusal, got {other:?}"),
    }

    // And the refusal changed nothing.
    assert_eq!(
        context_identity(&pool, context).await,
        (NEW_NAME.to_string(), NEW_SLUG.to_string()),
        "a refused rename leaves the identity pair untouched"
    );
}
