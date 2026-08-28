#![cfg(feature = "test-db")]
//! Integration tests for agent exercise: claim attribution (Beat A) and `vw_agent_exercise` (Beat B).
//! Each test runs on an isolated `#[sqlx::test]` database with the workspace migrations applied.

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
