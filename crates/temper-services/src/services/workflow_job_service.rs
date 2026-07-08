//! Thin wrappers over the `kb_workflow_jobs` SQL primitives (goal 019f3220). `DbBackend` composes
//! these into the dispatch tick; tests exercise them directly. Auth is NOT here — these are queue
//! primitives; the dispatch command that composes them carries the auth gate.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
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

/// Claim up to `limit` claimable jobs, leasing each for `lease_seconds`.
pub async fn claim(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
    lease_seconds: i32,
) -> ApiResult<Vec<ClaimedJob>> {
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!: Uuid", cogmap_id AS "cogmap_id!: Uuid", attempts AS "attempts!: i32"
          FROM workflow_job_claim($1, $2, $3, $4)
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
        let claimed = claim(&pool, "steward", "steward", 10, 600).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].cogmap_id, c);
        assert_eq!(claimed[0].attempts, 1, "attempts incremented at claim");
        assert_eq!(status_of(&pool, claimed[0].id).await, "in_progress");
        // A second claim finds nothing — it is no longer claimable.
        let again = claim(&pool, "steward", "steward", 10, 600).await.unwrap();
        assert!(again.is_empty(), "in_progress is not re-claimable");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn complete_marks_done_and_frees_the_slot(pool: PgPool) {
        let c = a_cogmap(&pool).await;
        enqueue(&pool, c, "steward", "steward").await.unwrap();
        claim(&pool, "steward", "steward", 10, 600).await.unwrap();
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
        let claimed = claim(&pool, "steward", "steward", 10, -1).await.unwrap();
        let id = claimed[0].id;
        // attempts=1, max=3 → reap sends it to waiting_for_retry.
        assert_eq!(reap(&pool, "boom").await.unwrap(), 1);
        assert_eq!(status_of(&pool, id).await, "waiting_for_retry");
        // Two more claim+reap cycles (attempts 2, then 3) → dead at attempts >= max_attempts.
        claim(&pool, "steward", "steward", 10, -1).await.unwrap();
        reap(&pool, "boom").await.unwrap();
        claim(&pool, "steward", "steward", 10, -1).await.unwrap();
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
            claim(&pool, "embed", "embed", 10, 600).await.unwrap().is_empty(),
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
}
