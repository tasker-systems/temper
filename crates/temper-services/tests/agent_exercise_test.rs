#![cfg(feature = "test-db")]
//! Integration tests for agent exercise: claim attribution (Beat A) and `vw_agent_exercise` (Beat B).
//! Each test runs on an isolated `#[sqlx::test]` database with the workspace migrations applied.

use chrono::{DateTime, Utc};
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

/// A principal that has authenticated but never claimed is visible, with the later rungs empty.
///
/// This is the population an inner join would drop, and it is the shape the #809 case actually takes:
/// reached, then nothing. It is also the shape a healthy idle agent takes, which is why the view
/// reports the rungs rather than judging them.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_view_shows_a_principal_that_reached_but_never_claimed(pool: PgPool) {
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
            Option<String>,
            Option<DateTime<Utc>>,
        ),
    >(
        "SELECT last_seen_at, last_claim_at, last_persona_claimed, last_emitted_at
           FROM vw_agent_exercise WHERE profile_id = $1",
    )
    .bind(*principal)
    .fetch_one(&pool)
    .await
    .expect("the principal is visible despite never having claimed");

    assert!(row.0.is_some(), "it reached");
    assert!(row.1.is_none(), "but claimed nothing");
    assert!(row.2.is_none(), "so no persona is observed");
    assert!(row.3.is_none(), "and nothing moved");
}

/// The claim rung fills in for a steward claim, and reports the persona it observed.
///
/// This is what Beat A bought: before it, `claimed_by_profile_id` was NULL for every steward job and
/// this row's rung 2 would be empty no matter how much the steward ran.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_view_fills_the_claim_rung_for_a_steward_claim(pool: PgPool) {
    let (principal, cogmap) = seed_steward_reach(&pool).await;
    seed_machine_client(&pool, principal).await;

    sqlx::query("SELECT workflow_job_enqueue($1, 'steward', 'steward')")
        .bind(cogmap)
        .execute(&pool)
        .await
        .unwrap();
    temper_services::services::workflow_job_service::claim(
        &pool, "steward", "steward", 10, 600, None, principal,
    )
    .await
    .expect("the claim succeeds");

    let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<String>)>(
        "SELECT last_claim_at, last_persona_claimed FROM vw_agent_exercise WHERE profile_id = $1",
    )
    .bind(*principal)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(row.0.is_some(), "the claim rung is filled");
    assert_eq!(
        row.1.as_deref(),
        Some("steward"),
        "and the observed persona is the steward's"
    );
}
