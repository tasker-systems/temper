//! Thin wrappers over the `kb_workflow_jobs` SQL primitives (goal 019f3220). `DbBackend` composes
//! these into the dispatch tick; tests exercise them directly. Auth is NOT here — these are queue
//! primitives; the dispatch command that composes them carries the auth gate.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use temper_core::types::ids::CorrelationId;
use temper_core::types::workflow_job::{ClaimedEmbedJob, ClaimedJob};

/// Enqueue a job for `(cogmap, persona, dispatch_type)`. Returns `Some(id)` when a new row was
/// created, `None` when one is already in-flight for the tuple (the single-flight dedup).
pub async fn enqueue(
    pool: &PgPool,
    cogmap_id: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_enqueue($1, $2, $3, '{}'::jsonb) AS "id: Uuid""#,
        cogmap_id,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Claim up to `limit` claimable jobs, leasing each for `lease_seconds`, stamping each claimed row
/// with the `correlation` of the dispatch tick that claimed it (`None` → NULL; the tick sent no
/// `x-steward-correlation-id`). The stamp is what `invocation_open` later inherits, so a session's
/// invocation joins back to its tick without the agent threading anything.
pub async fn claim(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
    lease_seconds: i32,
    correlation: Option<CorrelationId>,
) -> ApiResult<Vec<ClaimedJob>> {
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!: Uuid", cogmap_id AS "cogmap_id!: Uuid", attempts AS "attempts!: i32"
          FROM workflow_job_claim($1, $2, $3, $4, $5)
        "#,
        persona,
        dispatch_type,
        limit,
        lease_seconds,
        correlation.map(|c| c.uuid()),
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ClaimedJob {
            id: r.id,
            cogmap_id: r.cogmap_id,
            attempts: r.attempts,
        })
        .collect())
}

/// Transition the one active job for the tuple → done. Returns the job id if one was active.
pub async fn complete(
    pool: &PgPool,
    cogmap_id: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_complete($1, $2, $3) AS "id: Uuid""#,
        cogmap_id,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Enqueue a resource-keyed job for `(resource, persona, dispatch_type)` — the resource twin of
/// [`enqueue`]. Returns `Some(id)` when a new row was created, `None` when one is already in-flight
/// for the tuple (the single-flight dedup, which also gives supersede-on-update for embed jobs).
pub async fn enqueue_resource(
    pool: &PgPool,
    resource_id: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_enqueue_resource($1, $2, $3, '{}'::jsonb) AS "id: Uuid""#,
        resource_id,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Claim up to `limit` claimable resource-keyed jobs, leasing each for `lease_seconds` — the
/// resource twin of [`claim`], returning each job's `resource_id` (the embed scope).
pub async fn claim_resource(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
    lease_seconds: i32,
) -> ApiResult<Vec<ClaimedEmbedJob>> {
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!: Uuid", resource_id AS "resource_id!: Uuid", attempts AS "attempts!: i32"
          FROM workflow_job_claim_resource($1, $2, $3, $4)
        "#,
        persona,
        dispatch_type,
        limit,
        lease_seconds,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ClaimedEmbedJob {
            id: r.id,
            resource_id: r.resource_id,
            attempts: r.attempts,
        })
        .collect())
}

/// Transition the one active resource-keyed job for the tuple → done — the resource twin of
/// [`complete`]. Returns the job id if one was active.
pub async fn complete_resource(
    pool: &PgPool,
    resource_id: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_complete_resource($1, $2, $3) AS "id: Uuid""#,
        resource_id,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Re-drive `dead` resource-keyed jobs — re-enqueue a fresh pending job per resource that has a dead
/// job, up to `limit` resources. Returns the re-enqueued resource ids. Skips any resource that already
/// has a live job (the underlying `ON CONFLICT DO NOTHING` against the resource in-flight index), so a
/// re-drive never creates a duplicate active job. The dead rows are left as an accountability trail.
pub async fn redrive_resource(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
) -> ApiResult<Vec<Uuid>> {
    let rows = sqlx::query!(
        r#"
        SELECT resource_id AS "resource_id!: Uuid"
          FROM workflow_job_redrive_resource($1, $2, $3)
        "#,
        persona,
        dispatch_type,
        limit,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.resource_id).collect())
}

/// Reap expired-lease jobs → retry (or dead at max attempts). Returns the count reaped.
pub async fn reap(pool: &PgPool, error: &str) -> ApiResult<i32> {
    let n = sqlx::query_scalar!(r#"SELECT workflow_job_reap($1) AS "n!: i32""#, error)
        .fetch_one(pool)
        .await?;
    Ok(n)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
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
        enqueue(&pool, c, "steward", "steward").await.unwrap();
        let claimed = claim(&pool, "steward", "steward", 10, 600, None)
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].cogmap_id, c);
        assert_eq!(claimed[0].attempts, 1, "attempts incremented at claim");
        assert_eq!(status_of(&pool, claimed[0].id).await, "in_progress");
        // A second claim finds nothing — it is no longer claimable.
        let again = claim(&pool, "steward", "steward", 10, 600, None)
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
        enqueue(&pool, c, "steward", "steward").await.unwrap();
        // The steward cron mints a v4 uuid per tick; CorrelationId carries whatever the caller sends
        // (the column has no version requirement — see the design doc's "do not fix it to v7" note).
        let tick = CorrelationId::new();
        let claimed = claim(&pool, "steward", "steward", 10, 600, Some(tick))
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
        enqueue(&pool, c, "steward", "steward").await.unwrap();
        let claimed = claim(&pool, "steward", "steward", 10, 600, None)
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
        enqueue(&pool, c, "steward", "steward").await.unwrap();
        let first = CorrelationId::new();
        let claimed = claim(&pool, "steward", "steward", 10, -1, Some(first))
            .await
            .unwrap();
        let id = claimed[0].id;
        reap(&pool, "lease expired").await.unwrap();

        // Re-claim with an expiring lease again, so the row can be reaped once more below.
        let second = CorrelationId::new();
        claim(&pool, "steward", "steward", 10, -1, Some(second))
            .await
            .unwrap();
        assert_eq!(correlation_of(&pool, id).await, Some(second.uuid()));

        // …and a subsequent uncorrelated re-claim clears it rather than preserving the stale id.
        // (attempts is 2 here, below max_attempts=3, so the reap retries rather than killing it.)
        reap(&pool, "lease expired").await.unwrap();
        claim(&pool, "steward", "steward", 10, 600, None)
            .await
            .unwrap();
        assert_eq!(correlation_of(&pool, id).await, None);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn complete_marks_done_and_frees_the_slot(pool: PgPool) {
        let c = a_cogmap(&pool).await;
        enqueue(&pool, c, "steward", "steward").await.unwrap();
        claim(&pool, "steward", "steward", 10, 600, None)
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn reap_expired_lease_retries_then_dead(pool: PgPool) {
        let c = a_cogmap(&pool).await;
        enqueue(&pool, c, "steward", "steward").await.unwrap();
        // Claim with an already-past lease (negative seconds → lease_expires_at in the past).
        let claimed = claim(&pool, "steward", "steward", 10, -1, None)
            .await
            .unwrap();
        let id = claimed[0].id;
        // attempts=1, max=3 → reap sends it to waiting_for_retry.
        assert_eq!(reap(&pool, "boom").await.unwrap(), 1);
        assert_eq!(status_of(&pool, id).await, "waiting_for_retry");
        // Two more claim+reap cycles (attempts 2, then 3) → dead at attempts >= max_attempts.
        claim(&pool, "steward", "steward", 10, -1, None)
            .await
            .unwrap();
        reap(&pool, "boom").await.unwrap();
        claim(&pool, "steward", "steward", 10, -1, None)
            .await
            .unwrap();
        reap(&pool, "boom").await.unwrap();
        assert_eq!(
            status_of(&pool, id).await,
            "dead",
            "attempts hit max_attempts → dead"
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
        // A steward claim never picks up a resource-keyed job (disjoint scopes).
        assert!(
            claim(&pool, "embed", "embed", 10, 600, None)
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
}
