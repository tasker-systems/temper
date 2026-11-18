use super::*;

async fn a_cogmap(pool: &PgPool) -> Uuid {
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('m', $1) RETURNING id",
    )
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn status_of(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM kb_workflow_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn enqueue_dedup_keeps_one_active(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let first = enqueue(&pool, c, "steward", "steward").await.unwrap();
    let second = enqueue(&pool, c, "steward", "steward").await.unwrap();
    assert!(first.is_some(), "first enqueue creates a row");
    assert!(
        second.is_none(),
        "second is a no-op while the first is in-flight"
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_workflow_jobs WHERE cogmap_id = $1")
            .bind(c)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn claim_leases_and_increments_attempts(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let principal = a_profile(&pool, "wjs-claim-leases").await;
    reach(&pool, c, principal, "wjs-claim-leases-team").await;
    enqueue(&pool, c, "steward", "steward").await.unwrap();
    let claimed = claim(&pool, "steward", "steward", 10, 600, None, principal)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].cogmap_id, c);
    assert_eq!(claimed[0].attempts, 1, "attempts incremented at claim");
    assert_eq!(status_of(&pool, claimed[0].id).await, "in_progress");
    // A second claim finds nothing — it is no longer claimable.
    let again = claim(&pool, "steward", "steward", 10, 600, None, principal)
        .await
        .unwrap();
    assert!(again.is_empty(), "in_progress is not re-claimable");
}

async fn correlation_of(pool: &PgPool, id: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT correlation_id FROM kb_workflow_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn claim_stamps_the_tick_correlation(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let principal = a_profile(&pool, "wjs-claim-correlation").await;
    reach(&pool, c, principal, "wjs-claim-correlation-team").await;
    enqueue(&pool, c, "steward", "steward").await.unwrap();
    // The steward cron mints a v4 uuid per tick; CorrelationId carries whatever the caller sends
    // (the column has no version requirement — see the design doc's "do not fix it to v7" note).
    let tick = CorrelationId::new();
    let claimed = claim(&pool, "steward", "steward", 10, 600, Some(tick), principal)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(
        correlation_of(&pool, claimed[0].id).await,
        Some(tick.uuid()),
        "the claimed row records the tick that claimed it"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn claim_without_a_correlation_leaves_it_null(pool: PgPool) {
    // A caller that sends no `x-steward-correlation-id` claims exactly as before — NULL, never an
    // error. Correlation is provenance; nothing gates on it.
    let c = a_cogmap(&pool).await;
    let principal = a_profile(&pool, "wjs-claim-no-correlation").await;
    reach(&pool, c, principal, "wjs-claim-no-correlation-team").await;
    enqueue(&pool, c, "steward", "steward").await.unwrap();
    let claimed = claim(&pool, "steward", "steward", 10, 600, None, principal)
        .await
        .unwrap();
    assert_eq!(correlation_of(&pool, claimed[0].id).await, None);
}

#[sqlx::test(migrations = "../../migrations")]
async fn reclaim_after_reap_rebinds_to_the_new_tick(pool: PgPool) {
    // A job whose lease expired is requeued and re-claimed by a LATER tick. It must record the tick
    // that actually claimed it, not the one that lost it — otherwise a crashed tick's id would
    // haunt every subsequent retry, and the invocation would inherit a correlation whose logs
    // describe a different run.
    let c = a_cogmap(&pool).await;
    let principal = a_profile(&pool, "wjs-reclaim-after-reap").await;
    reach(&pool, c, principal, "wjs-reclaim-after-reap-team").await;
    enqueue(&pool, c, "steward", "steward").await.unwrap();
    let first = CorrelationId::new();
    let claimed = claim(&pool, "steward", "steward", 10, -1, Some(first), principal)
        .await
        .unwrap();
    let id = claimed[0].id;
    reap(&pool, "lease expired").await.unwrap();
    // Since `20260828000030` a reap defers the row; this test is about correlation rebinding,
    // not about timing, so let the backoff elapse rather than assert around it.
    elapse_backoff(&pool).await;

    // Re-claim with an expiring lease again, so the row can be reaped once more below.
    let second = CorrelationId::new();
    claim(&pool, "steward", "steward", 10, -1, Some(second), principal)
        .await
        .unwrap();
    assert_eq!(correlation_of(&pool, id).await, Some(second.uuid()));

    // …and a subsequent uncorrelated re-claim clears it rather than preserving the stale id.
    // (attempts is 2 here, below max_attempts=3, so the reap retries rather than killing it.)
    reap(&pool, "lease expired").await.unwrap();
    elapse_backoff(&pool).await;
    claim(&pool, "steward", "steward", 10, 600, None, principal)
        .await
        .unwrap();
    assert_eq!(correlation_of(&pool, id).await, None);
}

#[sqlx::test(migrations = "../../migrations")]
async fn complete_marks_done_and_frees_the_slot(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let principal = a_profile(&pool, "wjs-complete-frees-slot").await;
    reach(&pool, c, principal, "wjs-complete-frees-slot-team").await;
    enqueue(&pool, c, "steward", "steward").await.unwrap();
    claim(&pool, "steward", "steward", 10, 600, None, principal)
        .await
        .unwrap();
    let done = complete(&pool, c, "steward", "steward").await.unwrap();
    assert!(done.is_some());
    // Slot freed: a fresh drift episode can enqueue again.
    let reenq = enqueue(&pool, c, "steward", "steward").await.unwrap();
    assert!(
        reenq.is_some(),
        "done row does not block the in-flight index"
    );
}

// ── the scoped claim, and the completion that follows it ──────────────────

async fn a_profile(pool: &PgPool, handle: &str) -> ProfileId {
    let id: Uuid = crate::test_support::profile::insert(pool, handle).await;
    ProfileId::from(id)
}

/// Join `principal` to a team joined to `cogmap` — the only path
/// `cogmap_readable_by_profile` (and therefore `steward_candidate_cogmaps`, and therefore the
/// scoped claim) recognizes. Without this a scoped `claim_audit` returns NOTHING, which is
/// exactly the protection under test elsewhere and pure noise here.
async fn reach(pool: &PgPool, cogmap: Uuid, principal: ProfileId, slug: &str) {
    let team: Uuid = crate::test_support::team::insert(pool, slug).await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(*principal)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();
}

/// **THE CRITICAL BITE TEST, at the primitive.** Two cogmaps, one claimable auditor job each,
/// and a principal that reaches only one of them. The unreachable job must be invisible to its
/// claim — because the claim RETURNS THE PAYLOAD, so claiming it is simultaneously a disclosure
/// of another tenant's finding-id list and a theft of the job from the principal that enqueued
/// it. Against the shipped `workflow_job_claim` (filtering `(persona, dispatch_type, status)`
/// alone) this test claims both rows and reds on the length assertion.
///
/// It cannot pass vacuously: the reachable job is asserted claimed in the same call, so an empty
/// result — the trivial way to "not claim the other one" — fails too.
#[sqlx::test(migrations = "../../migrations")]
async fn claim_audit_never_claims_a_job_on_an_unreachable_cogmap(pool: PgPool) {
    let mine = a_cogmap(&pool).await;
    let theirs = a_cogmap(&pool).await;
    let auditor = a_profile(&pool, "wjs-scoped").await;
    reach(&pool, mine, auditor, "wjs-scoped-team").await;

    for c in [mine, theirs] {
        enqueue_with_payload(
            &pool,
            c,
            "auditor",
            "citation-audit",
            serde_json::json!({ "findings": [Uuid::now_v7()] }),
        )
        .await
        .unwrap()
        .expect("enqueued");
    }

    let claimed = claim_audit(&pool, "auditor", "citation-audit", 100, 600, None, auditor)
        .await
        .unwrap();

    assert_eq!(
        claimed.iter().map(|j| j.cogmap_id).collect::<Vec<_>>(),
        vec![mine],
        "the claim must hand back ONLY the cogmap this principal can reach — the other tenant's \
         job carries their finding ids in its payload and their pipeline in its slot"
    );
    let theirs_status: String = sqlx::query_scalar(
        "SELECT status FROM kb_workflow_jobs WHERE cogmap_id = $1 AND persona = 'auditor'",
    )
    .bind(theirs)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        theirs_status, "pending",
        "and it is still claimable by its own principal, not leased away and left to die at \
         max_attempts"
    );
}

/// The happy path — and it is the control for the two refusals below: same cogmap, same tuple,
/// only the claimant differs.
#[sqlx::test(migrations = "../../migrations")]
async fn complete_claimed_completes_the_job_this_principal_claimed(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let auditor = a_profile(&pool, "wjs-auditor").await;
    reach(&pool, c, auditor, "wjs-team").await;
    // A citation-audit job ALWAYS carries its cogmap's finding list — that is the grain fix
    // (one job per cogmap, N findings inside), and `claim_audit` deserializes it strictly. The
    // bare `enqueue` helper writes `{}`, which is not a shape the producer can emit, so using it
    // here failed with "missing field `findings`" rather than testing what it meant to.
    enqueue_with_payload(
        &pool,
        c,
        "auditor",
        "citation-audit",
        serde_json::json!({ "findings": [Uuid::now_v7()] }),
    )
    .await
    .unwrap();
    // A principal-less claim is no longer reachable through the Rust `claim` wrapper after Beat A
    // (its `principal` parameter is now mandatory, exactly as `claim_audit`'s always was), so this
    // constructs the pre-Beat-A "unscoped claim" scenario directly against the SQL primitive,
    // which still accepts `p_principal DEFAULT NULL` for callers outside this crate. The row it
    // leaves is claimed by nobody, and `complete_claimed` must decline it — proving the predicate
    // is the CLAIMANT, not merely "some in-flight job exists".
    let claimed_id: Uuid = sqlx::query_scalar("SELECT id FROM workflow_job_claim($1, $2, $3, $4)")
        .bind("auditor")
        .bind("citation-audit")
        .bind(10i32)
        .bind(600i32)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        complete_claimed(&pool, c, "auditor", "citation-audit", auditor)
            .await
            .unwrap()
            .is_none(),
        "an unclaimed-by-anyone in-flight job is not this principal's to complete"
    );
    assert_eq!(status_of(&pool, claimed_id).await, "in_progress");

    // Re-claim it under the auditor (reap first so it is claimable again).
    sqlx::query("UPDATE kb_workflow_jobs SET lease_expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    reap(&pool, "lease expired").await.unwrap();
    elapse_backoff(&pool).await;
    let mine = claim_audit(&pool, "auditor", "citation-audit", 10, 600, None, auditor)
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);

    let done = complete_claimed(&pool, c, "auditor", "citation-audit", auditor)
        .await
        .unwrap();
    assert_eq!(done, Some(mine[0].id));
    assert_eq!(status_of(&pool, mine[0].id).await, "done");
}

/// **A `pending` job is untouchable.** `workflow_job_complete` transitions
/// `('pending','in_progress','waiting_for_retry')`, so under the shipped primitive a caller
/// could terminate a job that had never been dispatched — suppressing a cogmap's auditing
/// indefinitely, with the sweep re-offering the finding and the completion removing it forever.
#[sqlx::test(migrations = "../../migrations")]
async fn complete_claimed_never_touches_a_pending_job(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let auditor = a_profile(&pool, "wjs-auditor").await;
    let job = enqueue(&pool, c, "auditor", "citation-audit")
        .await
        .unwrap()
        .expect("enqueued");

    assert!(
        complete_claimed(&pool, c, "auditor", "citation-audit", auditor)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        status_of(&pool, job).await,
        "pending",
        "a never-dispatched job must survive a completion attempt"
    );
    // …and the shipped primitive really would have taken it, which is why the narrowing exists.
    assert!(complete(&pool, c, "auditor", "citation-audit")
        .await
        .unwrap()
        .is_some());
}

/// **Another principal's in-flight job is untouchable.** Completing it early frees the
/// single-flight slot while that session is still running, so the next tick enqueues and claims
/// a second concurrent audit over the same finding list — into a trail that cannot retract.
#[sqlx::test(migrations = "../../migrations")]
async fn complete_claimed_never_touches_another_principals_in_flight_job(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let mine = a_profile(&pool, "wjs-mine").await;
    let theirs = a_profile(&pool, "wjs-theirs").await;
    reach(&pool, c, theirs, "wjs-theirs-team").await;
    // A citation-audit job ALWAYS carries its cogmap's finding list — that is the grain fix
    // (one job per cogmap, N findings inside), and `claim_audit` deserializes it strictly. The
    // bare `enqueue` helper writes `{}`, which is not a shape the producer can emit, so using it
    // here failed with "missing field `findings`" rather than testing what it meant to.
    enqueue_with_payload(
        &pool,
        c,
        "auditor",
        "citation-audit",
        serde_json::json!({ "findings": [Uuid::now_v7()] }),
    )
    .await
    .unwrap();
    let claimed = claim_audit(&pool, "auditor", "citation-audit", 10, 600, None, theirs)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);

    assert!(
        complete_claimed(&pool, c, "auditor", "citation-audit", mine)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        status_of(&pool, claimed[0].id).await,
        "in_progress",
        "the other session keeps its slot"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn reap_expired_lease_retries_then_dead(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let principal = a_profile(&pool, "wjs-reap-retries-then-dead").await;
    reach(&pool, c, principal, "wjs-reap-retries-then-dead-team").await;
    enqueue(&pool, c, "steward", "steward").await.unwrap();
    // Claim with an already-past lease (negative seconds → lease_expires_at in the past).
    let claimed = claim(&pool, "steward", "steward", 10, -1, None, principal)
        .await
        .unwrap();
    let id = claimed[0].id;
    // attempts=1, max=3 → reap sends it to waiting_for_retry.
    assert_eq!(reap(&pool, "boom").await.unwrap(), 1);
    assert_eq!(status_of(&pool, id).await, "waiting_for_retry");
    // Two more claim+reap cycles (attempts 2, then 3) → dead at attempts >= max_attempts.
    // Each re-claim needs the `20260828000030` backoff to have elapsed; this test asserts the
    // attempt ladder, and `the_backoff_grows_with_attempts_and_is_capped` asserts the delay.
    elapse_backoff(&pool).await;
    claim(&pool, "steward", "steward", 10, -1, None, principal)
        .await
        .unwrap();
    reap(&pool, "boom").await.unwrap();
    elapse_backoff(&pool).await;
    claim(&pool, "steward", "steward", 10, -1, None, principal)
        .await
        .unwrap();
    reap(&pool, "boom").await.unwrap();
    assert_eq!(
        status_of(&pool, id).await,
        "dead",
        "attempts hit max_attempts → dead"
    );
}

/// Simulate the retry backoff elapsing, so a test about something OTHER than timing can
/// re-claim. `20260828000030` pushes `next_visible_at` forward on every retry, and every claim
/// variant filters `next_visible_at <= now()` — so without this, a reap-then-claim sequence
/// silently claims nothing and the assertion that follows it fails for the wrong reason.
async fn elapse_backoff(pool: &PgPool) {
    sqlx::query("UPDATE kb_workflow_jobs SET next_visible_at = now()")
        .execute(pool)
        .await
        .unwrap();
}

/// **A REAPED JOB MUST NOT BE INSTANTLY RE-CLAIMABLE.** `next_visible_at` has existed since
/// `20260705000001` and every claim variant filters on it, but until `20260828000030` NOTHING
/// EVER WROTE IT — so the column, its index (`idx_workflow_jobs_claimable`) and the filter
/// described a backoff the system did not perform.
///
/// The cost was measured: `embed` (four shards) and `region` both tick `* * * * *`, so a job
/// failing against a persistently-unavailable upstream burned all three attempts as fast as the
/// lease could expire, then died and was re-enqueued to do it again.
#[sqlx::test(migrations = "../../migrations")]
async fn a_reaped_job_is_not_immediately_claimable_again(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let principal = a_profile(&pool, "wjs-reaped-not-immediately-claimable").await;
    reach(
        &pool,
        c,
        principal,
        "wjs-reaped-not-immediately-claimable-team",
    )
    .await;
    enqueue(&pool, c, "steward", "steward").await.unwrap();
    let claimed = claim(&pool, "steward", "steward", 10, -1, None, principal)
        .await
        .unwrap();
    let id = claimed[0].id;

    reap(&pool, "lease expired").await.unwrap();
    assert_eq!(status_of(&pool, id).await, "waiting_for_retry");

    let deferred: bool =
        sqlx::query_scalar("SELECT next_visible_at > now() FROM kb_workflow_jobs WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        deferred,
        "reap must push next_visible_at forward, or the retry is immediate"
    );

    // The column moving is not the same claim as the BEHAVIOUR changing. This is the half that
    // would still fail if a future claim variant forgot the `next_visible_at <= now()` filter.
    let too_soon = claim(&pool, "steward", "steward", 10, 600, None, principal)
        .await
        .unwrap();
    assert!(
        too_soon.iter().all(|j| j.id != id),
        "a backed-off job must not be claimable yet"
    );

    elapse_backoff(&pool).await;
    let now_claimable = claim(&pool, "steward", "steward", 10, 600, None, principal)
        .await
        .unwrap();
    assert!(
        now_claimable.iter().any(|j| j.id == id),
        "once the backoff elapses the job must be claimable again — a backoff that never \
         clears is a leak, not a delay"
    );
}

/// **THE BACKOFF GROWS, AND IT IS BOUNDED.** An unbounded doubling on a queue whose jobs are
/// re-created by their own sweep every tick is a leak rather than a backoff, so the curve is
/// capped. The cap does not bind at `max_attempts = 3`; it exists so raising that constant
/// stays safe, which is exactly the kind of change nobody re-derives a bound for.
#[sqlx::test(migrations = "../../migrations")]
async fn the_backoff_grows_with_attempts_and_is_capped(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let principal = a_profile(&pool, "wjs-backoff-grows-and-capped").await;
    reach(&pool, c, principal, "wjs-backoff-grows-and-capped-team").await;
    enqueue(&pool, c, "steward", "steward").await.unwrap();

    let mut delays: Vec<f64> = Vec::new();
    for _ in 0..2 {
        claim(&pool, "steward", "steward", 10, -1, None, principal)
            .await
            .unwrap();
        reap(&pool, "lease expired").await.unwrap();
        let secs: f64 = sqlx::query_scalar(
            "SELECT extract(epoch FROM (next_visible_at - now()))::float8 \
               FROM kb_workflow_jobs WHERE cogmap_id = $1",
        )
        .bind(c)
        .fetch_one(&pool)
        .await
        .unwrap();
        delays.push(secs);
        elapse_backoff(&pool).await;
    }

    assert!(
        delays[1] > delays[0],
        "the second retry must wait longer than the first: {delays:?}"
    );
    assert!(
        delays.iter().all(|d| *d <= 3600.0),
        "no retry may be deferred past the cap: {delays:?}"
    );
    assert!(
        delays[0] >= 60.0,
        "the first retry must be a real gap against a one-minute cron, not a rounding error: \
         {delays:?}"
    );
}

/// **A DYING JOB IS NOT DELAYED.** The two arms are independent: backoff belongs to
/// `waiting_for_retry`, and a job at `max_attempts` goes `dead` and `completed_at` immediately,
/// exactly as `20260705000001` had it. Deferring a terminal row would leave it looking
/// scheduled forever.
#[sqlx::test(migrations = "../../migrations")]
async fn backoff_does_not_defer_a_job_that_dies(pool: PgPool) {
    let c = a_cogmap(&pool).await;
    let principal = a_profile(&pool, "wjs-backoff-no-defer-on-death").await;
    reach(&pool, c, principal, "wjs-backoff-no-defer-on-death-team").await;
    enqueue(&pool, c, "steward", "steward").await.unwrap();
    let id = claim(&pool, "steward", "steward", 10, -1, None, principal)
        .await
        .unwrap()[0]
        .id;

    for _ in 0..2 {
        reap(&pool, "boom").await.unwrap();
        elapse_backoff(&pool).await;
        claim(&pool, "steward", "steward", 10, -1, None, principal)
            .await
            .unwrap();
    }
    reap(&pool, "boom").await.unwrap();

    let (status, completed, deferred): (String, bool, bool) = sqlx::query_as(
        "SELECT status, completed_at IS NOT NULL, next_visible_at > now() \
           FROM kb_workflow_jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(status, "dead", "attempts hit max_attempts → dead");
    assert!(completed, "a dead job is stamped completed_at");
    assert!(
        !deferred,
        "a dead job must not carry a future next_visible_at — it is terminal, not scheduled"
    );
}

// ── resource-keyed (embed) queue ──────────────────────────────────────────

async fn a_resource(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('doc', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn resource_enqueue_dedup_keeps_one_active(pool: PgPool) {
    let r = a_resource(&pool).await;
    let first = enqueue_resource(&pool, r, "embed", "embed").await.unwrap();
    let second = enqueue_resource(&pool, r, "embed", "embed").await.unwrap();
    assert!(first.is_some(), "first enqueue creates a row");
    assert!(
        second.is_none(),
        "second is a no-op while the first is in-flight (supersede-safe single-flight)"
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_workflow_jobs WHERE resource_id = $1")
            .bind(r)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn resource_claim_returns_resource_id_and_completes(pool: PgPool) {
    let r = a_resource(&pool).await;
    enqueue_resource(&pool, r, "embed", "embed").await.unwrap();
    let claimed = claim_resource(&pool, "embed", "embed", 10, 600)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].resource_id, r, "claim surfaces the embed scope");
    assert_eq!(claimed[0].attempts, 1);
    assert_eq!(status_of(&pool, claimed[0].id).await, "in_progress");
    // A steward claim never picks up a resource-keyed job (disjoint scopes). The principal here
    // reaches nothing — irrelevant to this assertion, since a resource-keyed job is invisible to
    // the cogmap-claim regardless of scoping.
    let principal = a_profile(&pool, "wjs-resource-cogmap-disjoint").await;
    assert!(
        claim(&pool, "embed", "embed", 10, 600, None, principal)
            .await
            .unwrap()
            .is_empty(),
        "the cogmap-claim's cogmap_id RETURNING would choke on a resource job — scopes are disjoint"
    );
    // Complete frees the slot: a fresh episode can enqueue again.
    assert!(complete_resource(&pool, r, "embed", "embed")
        .await
        .unwrap()
        .is_some());
    assert!(
        enqueue_resource(&pool, r, "embed", "embed")
            .await
            .unwrap()
            .is_some(),
        "done row does not block the resource in-flight index"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn resource_and_cogmap_scopes_are_independent(pool: PgPool) {
    // Same table, different scopes: a cogmap job and a resource job coexist and don't dedup each
    // other. The global reaper covers both.
    let c = a_cogmap(&pool).await;
    let r = a_resource(&pool).await;
    assert!(enqueue(&pool, c, "steward", "steward")
        .await
        .unwrap()
        .is_some());
    assert!(enqueue_resource(&pool, r, "embed", "embed")
        .await
        .unwrap()
        .is_some());
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_workflow_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 2, "both scopes hold a row");
}

#[sqlx::test(migrations = "../../migrations")]
async fn redrive_resource_reenqueues_dead_and_skips_active(pool: PgPool) {
    let dead = a_resource(&pool).await;
    let dead_job = enqueue_resource(&pool, dead, "embed", "embed")
        .await
        .unwrap()
        .expect("enqueue");
    sqlx::query("UPDATE kb_workflow_jobs SET status = 'dead' WHERE id = $1")
        .bind(dead_job)
        .execute(&pool)
        .await
        .unwrap();

    // A resource with a live job must be untouched by re-drive (single-flight preserved).
    let live = a_resource(&pool).await;
    enqueue_resource(&pool, live, "embed", "embed")
        .await
        .unwrap()
        .expect("enqueue");

    let redriven = redrive_resource(&pool, "embed", "embed", 10).await.unwrap();
    assert_eq!(
        redriven,
        vec![dead],
        "only the dead-jobbed resource is re-driven"
    );

    // The dead row stays as history; a fresh pending row now exists for the same resource.
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_workflow_jobs WHERE resource_id = $1 AND status = 'pending'",
    )
    .bind(dead)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 1, "dead job re-enqueued as a new pending row");
    assert_eq!(
        status_of(&pool, dead_job).await,
        "dead",
        "dead row preserved"
    );

    // Idempotent while the re-driven job is live: a second pass re-drives nothing.
    assert!(redrive_resource(&pool, "embed", "embed", 10)
        .await
        .unwrap()
        .is_empty());
}
