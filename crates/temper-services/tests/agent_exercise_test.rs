#![cfg(feature = "test-db")]
//! Integration tests for agent exercise: claim attribution (Beat A) and `vw_agent_exercise` (Beat B).
//! Each test runs on an isolated `#[sqlx::test]` database with the workspace migrations applied.

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use temper_core::types::ProfileId;
use uuid::Uuid;

/// A fresh principal, team, and cogmap: `principal` is a MEMBER of `team`, and `team` is joined to
/// `cogmap` via `kb_team_cogmaps` — the only path `cogmap_readable_by_profile` recognizes, and
/// therefore the only path `steward_candidate_cogmaps` (and the scoped claim built on it) recognizes
/// too (`migrations/20260705000002_steward_drift_sweep.sql:11-17`,
/// `migrations/20260624000002_canonical_functions.sql:259-267`). Mirrors
/// `workflow_job_service`'s own `reach` test helper, duplicated here rather than shared because this
/// crate keeps no shared fixture crate under `tests/` — every file defines its own `seed_*`.
///
/// Every call mints an INDEPENDENT profile, team and cogmap, all named from one fresh
/// `Uuid::now_v7()`. `kb_profiles.handle` and `kb_teams.slug` are both UNIQUE, so a second call
/// reusing either would fail the insert outright — and reusing the TEAM would make both calls'
/// cogmaps mutually reachable, which would let
/// `a_claim_does_not_take_another_principals_queued_work` (Task A2) pass vacuously instead of
/// exercising the scoping it names.
async fn seed_steward_reach(pool: &PgPool) -> (ProfileId, Uuid) {
    let unique = Uuid::now_v7();

    let principal: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(format!("steward-reach-{unique}"))
    .fetch_one(pool)
    .await
    .unwrap();

    let team: Uuid =
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(format!("steward-reach-team-{unique}"))
            .fetch_one(pool)
            .await
            .unwrap();

    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(principal)
    .execute(pool)
    .await
    .unwrap();

    // The telos resource `kb_cogmaps.telos_resource_id` NOT NULL FK demands; otherwise irrelevant.
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id",
    )
    .bind(format!("steward-reach-telos-{unique}"))
    .fetch_one(pool)
    .await
    .unwrap();

    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("steward-reach-map-{unique}"))
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

    (ProfileId::from(principal), cogmap)
}

/// A steward claim records the principal that made it.
///
/// Before this, `claim` passed five arguments so `p_principal` defaulted NULL and every steward job
/// carried a NULL claimant — the asymmetry `workflow_job_service::claim_audit` documents.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_steward_claim_records_its_principal(pool: PgPool) {
    let (principal, cogmap) = seed_steward_reach(&pool).await;

    sqlx::query("SELECT workflow_job_enqueue($1, 'steward', 'steward')")
        .bind(cogmap)
        .execute(&pool)
        .await
        .unwrap();

    let claimed = temper_services::services::workflow_job_service::claim(
        &pool, "steward", "steward", 10, 600, None, principal,
    )
    .await
    .expect("the claim succeeds");
    assert_eq!(
        claimed.len(),
        1,
        "the enqueued job is claimable by its own principal"
    );

    let claimant: Option<Uuid> = sqlx::query_scalar(
        "SELECT claimed_by_profile_id FROM kb_workflow_jobs WHERE cogmap_id = $1",
    )
    .bind(cogmap)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        claimant,
        Some(*principal),
        "and the row records who claimed it"
    );
}

/// A steward tick claims its own principal's queued work and not a stranger's.
///
/// The bite: with `p_principal` NULL (the pre-A1 spelling) BOTH jobs are claimable, because the
/// claim filtered on `(persona, dispatch_type, status)` alone. This test fails if the scoping is
/// removed, which is the point of writing it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_claim_does_not_take_another_principals_queued_work(pool: PgPool) {
    let (mine, my_cogmap) = seed_steward_reach(&pool).await;
    let (_theirs, their_cogmap) = seed_steward_reach(&pool).await;

    for cogmap in [my_cogmap, their_cogmap] {
        sqlx::query("SELECT workflow_job_enqueue($1, 'steward', 'steward')")
            .bind(cogmap)
            .execute(&pool)
            .await
            .unwrap();
    }

    let claimed = temper_services::services::workflow_job_service::claim(
        &pool, "steward", "steward", 10, 600, None, mine,
    )
    .await
    .expect("the claim succeeds");

    assert_eq!(
        claimed.len(),
        1,
        "exactly one job is claimable by this principal"
    );
    assert_eq!(
        claimed[0].cogmap_id, my_cogmap,
        "and it is this principal's own, never the cogmap it cannot reach"
    );
}

/// Register `principal` in the `kb_machine_clients` allowlist and return the new row's `id`.
///
/// `client_id` is UNIQUE, so it is minted fresh per call from a `Uuid::now_v7()` rather than
/// derived from `principal` — unlike the sibling helpers in `standing_clock_test.rs` and
/// `citation_audit_handler_test.rs`, which key it off the profile because each of their tests
/// registers a given profile only once. `label` and `client_id` share the mint, exactly as those
/// two helpers share `client_id` and `label` off their own key.
async fn seed_machine_client(pool: &PgPool, principal: ProfileId) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, $1, $2, $2) RETURNING id",
    )
    .bind(format!("agent-exercise-{}", Uuid::now_v7()))
    .bind(*principal)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// An emitter entity owned by `principal` — the row both `kb_events.emitter_entity_id` and
/// `kb_invocations.scoped_entity_id` point at, and therefore the join `vw_agent_exercise` walks to
/// get from an event or a session back to the principal that produced it.
///
/// `kb_entities_profile_id_name_key` is UNIQUE on `(profile_id, name)`, so the name is minted fresh
/// per call. In production `writes::resolve_emitter` names one entity per surface off the profile's
/// handle; nothing here depends on that spelling, only on the FK.
async fn seed_entity(pool: &PgPool, principal: ProfileId) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id")
        .bind(*principal)
        .bind(format!("agent-exercise@{}", Uuid::now_v7()))
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Append one event of `category`, emitted by `entity`, and return its stored `occurred_at`.
///
/// The category is chosen on BOTH sides deliberately: `kb_events.category` defaults to `'domain'`
/// and the FK `kb_events_category_matches_type` pins it to the event type's own category, so an
/// admin event needs an admin type AND the explicit column. The timestamp comes back from
/// `RETURNING` rather than from `Utc::now()` in Rust, because Postgres stores microseconds and a
/// nanosecond-precision Rust value would not compare equal to what the view reads back.
async fn seed_event(
    pool: &PgPool,
    entity: Uuid,
    category: &str,
    occurred_at: DateTime<Utc>,
) -> (Uuid, DateTime<Utc>) {
    sqlx::query_as(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, category, occurred_at) \
         VALUES ((SELECT id FROM kb_event_types WHERE category = $1 ORDER BY name LIMIT 1), \
                 $2, $1, $3) \
         RETURNING id, occurred_at",
    )
    .bind(category)
    .bind(entity)
    .bind(occurred_at)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// One invocation envelope scoped to `entity`, opened by `opened_by_event` and already closed with
/// `status`.
///
/// `originating_cogmap_id`, `telos_resource_id` and `opened_by_event_id` are all `NOT NULL` with
/// FKs, so the envelope needs a real cogmap (the one `seed_steward_reach` minted), that cogmap's own
/// telos resource, and a real event. `closed_by_event_id` is nullable and deliberately left NULL:
/// the view reads `closed_at` and `status`, never the closing event.
async fn seed_closed_invocation(
    pool: &PgPool,
    entity: Uuid,
    cogmap: Uuid,
    opened_by_event: Uuid,
    status: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_invocations \
           (id, opened_by_event_id, status, trigger_kind, originating_cogmap_id, scoped_entity_id, \
            telos_resource_id, opened_at, closed_at) \
         SELECT $1, $2, $3, 'scheduled', c.id, $5, c.telos_resource_id, \
                now() - interval '5 minutes', now() \
           FROM kb_cogmaps c WHERE c.id = $4",
    )
    .bind(id)
    .bind(opened_by_event)
    .bind(status)
    .bind(cogmap)
    .bind(entity)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// A principal that has authenticated but never run is visible, with every later rung empty.
///
/// This is the population an inner join would drop, and it is the shape a quietly-skipped agent
/// actually takes: reached, then nothing. It is also the shape a healthy idle agent takes, which is
/// why the view reports the rungs rather than judging them.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_view_shows_a_principal_that_reached_but_never_ran(pool: PgPool) {
    let (principal, _cogmap) = seed_steward_reach(&pool).await;
    let client = seed_machine_client(&pool, principal).await;
    sqlx::query("UPDATE kb_machine_clients SET last_seen_at = now() WHERE id = $1")
        .bind(client)
        .execute(&pool)
        .await
        .unwrap();

    let row = sqlx::query_as::<
        _,
        (
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<String>,
            Option<DateTime<Utc>>,
        ),
    >(
        "SELECT last_seen_at, last_session_opened_at, last_session_closed_at, last_session_status,
                last_emitted_at
           FROM vw_agent_exercise WHERE profile_id = $1",
    )
    .bind(*principal)
    .fetch_one(&pool)
    .await
    .expect("the principal is visible despite never having run");

    assert!(row.0.is_some(), "it reached");
    assert!(row.1.is_none(), "but opened no session");
    assert!(row.2.is_none(), "so closed none either");
    assert!(row.3.is_none(), "and no session status is observed");
    assert!(row.4.is_none(), "and nothing moved");
}

/// The session rungs fill from an invocation the principal opened, and report how it ended.
///
/// Replaces the claim-rung test: the view no longer reads `kb_workflow_jobs.claimed_by_profile_id`,
/// because that column is overwritten in place when another principal claims a reaped job.
///
/// The bite this test exists for: point the sessions lateral at the wrong column — `en.id =
/// p.profile_id` instead of `en.profile_id = p.profile_id` — and both `uuid` columns still typecheck,
/// `CREATE VIEW` still succeeds, and rungs 2 and 3 are NULL for every principal forever. Every
/// assertion below fails in that world; nothing in the negative-shaped tests would.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_view_fills_the_session_rungs_from_an_invocation(pool: PgPool) {
    let (principal, cogmap) = seed_steward_reach(&pool).await;
    seed_machine_client(&pool, principal).await;
    let entity = seed_entity(&pool, principal).await;
    let (opened_by, _) = seed_event(&pool, entity, "domain", Utc::now()).await;
    seed_closed_invocation(&pool, entity, cogmap, opened_by, "completed").await;

    let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<String>)>(
        "SELECT last_session_opened_at, last_session_closed_at, last_session_status
           FROM vw_agent_exercise WHERE profile_id = $1",
    )
    .bind(*principal)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(row.0.is_some(), "the session rung is filled");
    assert!(row.1.is_some(), "and the session closed");
    assert_eq!(
        row.2.as_deref(),
        Some("completed"),
        "and the view reports how it ended"
    );
}

/// Rung 4 reports the principal's latest DOMAIN event, and an admin act does not stand in for one.
///
/// Two bites in one behaviour. Point the events lateral at `en.id = p.profile_id` and
/// `last_emitted_at` is NULL forever — the defect that survived every earlier test in this file,
/// because none of them ever emitted anything. Drop `AND ev.category = 'domain'` and the newer
/// admin event wins instead, so a principal that only ever performed administration reads as having
/// moved the corpus.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_view_reports_the_latest_domain_event_and_not_an_admin_one(pool: PgPool) {
    let (principal, _cogmap) = seed_steward_reach(&pool).await;
    seed_machine_client(&pool, principal).await;
    let entity = seed_entity(&pool, principal).await;

    let now = Utc::now();
    let (_, domain_at) = seed_event(&pool, entity, "domain", now - Duration::hours(1)).await;
    seed_event(&pool, entity, "admin", now).await;

    let emitted: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT last_emitted_at FROM vw_agent_exercise WHERE profile_id = $1")
            .bind(*principal)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(
        emitted,
        Some(domain_at),
        "rung 4 is the latest domain event, not the newer admin one"
    );
}

/// Several credential rows on one profile collapse to ONE view row, counted rather than repeated.
///
/// `kb_machine_clients` has no unique constraint on `profile_id`, and `20260711000010` states that
/// reactivation is a new registration and never an `UPDATE`, so a rebound or re-registered agent
/// holds two rows. Selecting straight from that table gave one view row per CREDENTIAL, each
/// repeating the profile-wide rungs — so a revoked credential reported sessions and corpus movement
/// dated after its own `revoked_at`. `fetch_all` rather than `fetch_one` here on purpose: the
/// duplication this pins would surface as a second row, and `fetch_one` reports that as an opaque
/// row-count error rather than as the count it actually is.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn two_credentials_on_one_profile_are_one_row_with_a_live_count(pool: PgPool) {
    let (principal, _cogmap) = seed_steward_reach(&pool).await;
    let revoked = seed_machine_client(&pool, principal).await;
    seed_machine_client(&pool, principal).await;
    sqlx::query("UPDATE kb_machine_clients SET revoked_at = now() WHERE id = $1")
        .bind(revoked)
        .execute(&pool)
        .await
        .unwrap();

    let rows = sqlx::query_as::<_, (i64, i64)>(
        "SELECT credentials, credentials_live FROM vw_agent_exercise WHERE profile_id = $1",
    )
    .bind(*principal)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 1, "one principal is exactly one row");
    assert_eq!(rows[0].0, 2, "both credential rows are counted");
    assert_eq!(rows[0].1, 1, "and only the unrevoked one is live");
}
