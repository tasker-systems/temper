//! The async-embedding drain (issue #299). One `dispatch_tick` pass reaps stale leases, claims a
//! batch of resource-keyed embed jobs, backfills each resource's deferred chunk embeddings
//! (`temper_substrate::embed::embed_resource_chunks`), and marks the clean ones done. A failed embed
//! is left `in_progress` for the reaper's retry→dead path — errors are off the create request path.
//!
//! SERVICE-DIRECT (not a `Backend` trait command): embedding is a **system** backfill with no profile
//! scoping — unlike the steward dispatch, which sweeps profile-visible maps. The `/api/embed/dispatch`
//! endpoint (CRON_SECRET-gated) is the only caller; there is no per-resource authz because the queue
//! is fed only by the trusted server-side write path.

use sqlx::PgPool;

use crate::error::ApiResult;
use crate::services::workflow_job_service;
use temper_core::types::workflow_job::{
    DispatchType, EmbedDispatchSummary, Persona, DEFAULT_EMBED_DISPATCH_CAP,
    DEFAULT_EMBED_LEASE_SECONDS,
};

/// Whether this deployment defers server-computed embeddings to the async drain (issue #299). Read
/// from `TEMPER_ASYNC_EMBED` (`1`/`true` ⇒ on) per call — a deployment toggle flipped on once the
/// embed-dispatch cron is confirmed running. Default **OFF**: server-computed embeds run inline
/// (exactly as before), so a deployment without the drain never strands chunks unembedded. Caller-
/// supplied `chunks_packed` (bring-your-own vectors) is never affected — there is nothing to defer.
pub fn async_embed_enabled() -> bool {
    std::env::var("TEMPER_ASYNC_EMBED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Run one embed-dispatch pass: reap → claim up to `cap` embed jobs → embed each resource → complete
/// the clean ones. Returns a [`EmbedDispatchSummary`] for cron/operator observability. `cap` defaults
/// to [`DEFAULT_EMBED_DISPATCH_CAP`] when `None`.
///
/// A per-job embed error is caught and tallied as `failed` (the job stays `in_progress`; the reaper
/// retries it, then marks it `dead` at max attempts) — one bad resource never aborts the pass or the
/// other jobs. The pass never returns an error for an embed failure; it only surfaces `ApiError` for a
/// genuine queue/DB fault.
pub async fn dispatch_tick(pool: &PgPool, cap: Option<i32>) -> ApiResult<EmbedDispatchSummary> {
    let persona = Persona::Embed.as_str();
    let dispatch = DispatchType::Embed.as_str();

    // Reap stale leases (a crashed embed → retry/dead) before claiming, mirroring the steward tick.
    workflow_job_service::reap(pool, "embed lease expired").await?;

    let cap = cap.unwrap_or(DEFAULT_EMBED_DISPATCH_CAP);
    let claimed = workflow_job_service::claim_resource(
        pool,
        persona,
        dispatch,
        cap,
        DEFAULT_EMBED_LEASE_SECONDS,
    )
    .await?;

    let mut summary = EmbedDispatchSummary {
        claimed: claimed.len() as u32,
        ..Default::default()
    };

    for job in claimed {
        match temper_substrate::embed::embed_resource_chunks(pool, job.resource_id).await {
            Ok(n) => {
                // Clean embed → complete the job (frees the single-flight slot).
                workflow_job_service::complete_resource(pool, job.resource_id, persona, dispatch)
                    .await?;
                summary.completed += 1;
                summary.chunks_embedded += n;
            }
            Err(e) => {
                // Leave the job in_progress; the reaper's lease-expiry sweep retries it (then dead at
                // max attempts). Observable via the summary + the job's last_error on reap.
                tracing::warn!(
                    resource_id = %job.resource_id,
                    attempts = job.attempts,
                    error = %e,
                    "embed job failed; leaving for reaper retry"
                );
                summary.failed += 1;
            }
        }
    }

    Ok(summary)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;

    async fn a_resource(pool: &PgPool) -> uuid::Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('embed-target', '') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn status_of(pool: &PgPool, id: uuid::Uuid) -> String {
        sqlx::query_scalar("SELECT status FROM kb_workflow_jobs WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    // The drain orchestration: an enqueued embed job is claimed, its (no-op, chunkless) embed
    // completes cleanly, the job flips to done, and the summary tallies it. A resource with no
    // deferred chunks embeds zero chunks — exercising reap→claim→embed→complete→summary without the
    // full chunk graph (the real ONNX round-trip is covered by the Phase 4 e2e create→drain→search
    // test). ONNX-free: embed_resource_chunks runs no inference when there are no NULL-vector chunks.
    #[sqlx::test(migrations = "../../migrations")]
    async fn dispatch_tick_claims_embeds_and_completes(pool: PgPool) {
        let r = a_resource(&pool).await;
        let job = workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
            .await
            .unwrap()
            .expect("enqueue creates a job");

        let summary = dispatch_tick(&pool, Some(5)).await.unwrap();
        assert_eq!(summary.claimed, 1);
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.chunks_embedded, 0, "no deferred chunks to embed");
        assert_eq!(
            status_of(&pool, job).await,
            "done",
            "clean embed → job done"
        );

        // Idempotent: a second tick finds nothing to claim.
        let again = dispatch_tick(&pool, Some(5)).await.unwrap();
        assert_eq!(again.claimed, 0);
    }

    // An empty queue is a clean no-op pass.
    #[sqlx::test(migrations = "../../migrations")]
    async fn dispatch_tick_empty_queue_is_noop(pool: PgPool) {
        let summary = dispatch_tick(&pool, None).await.unwrap();
        assert_eq!(summary.claimed, 0);
        assert_eq!(summary.completed, 0);
        assert_eq!(summary.chunks_embedded, 0);
    }
}
