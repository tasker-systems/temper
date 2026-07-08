//! The async-embedding drain (issue #299). One `dispatch_tick` pass reaps stale leases, claims a
//! batch of resource-keyed embed jobs, backfills each resource's deferred chunk embeddings
//! (`temper_substrate::embed::embed_resource_chunks`), and marks the clean ones done. A failed embed
//! is left `in_progress` for the reaper's retry→dead path — errors are off the create request path.
//!
//! SERVICE-DIRECT (not a `Backend` trait command): embedding is a **system** backfill with no profile
//! scoping — unlike the steward dispatch, which sweeps profile-visible maps. The `/api/embed/dispatch`
//! endpoint (CRON_SECRET-gated) is the only caller; there is no per-resource authz because the queue
//! is fed only by the trusted server-side write path.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::services::workflow_job_service;
use temper_core::types::workflow_job::{
    DispatchType, EmbedDispatchSummary, EmbeddingStatus, Persona, DEFAULT_EMBED_DISPATCH_CAP,
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

/// Re-enqueue `dead` embed jobs so a following claim can drain them (issue #299, Phase 4). Returns the
/// number of resources re-driven. `cap` bounds the resources re-driven per call (defaults to
/// [`DEFAULT_EMBED_DISPATCH_CAP`]). A resource that already has a live job is skipped — re-drive never
/// duplicates an active job. This is the recovery path for a resource stranded FTS-only by a failed
/// embed; the dead rows stay as an accountability trail.
pub async fn redrive(pool: &PgPool, cap: Option<i32>) -> ApiResult<u32> {
    let cap = cap.unwrap_or(DEFAULT_EMBED_DISPATCH_CAP);
    let redriven = workflow_job_service::redrive_resource(
        pool,
        Persona::Embed.as_str(),
        DispatchType::Embed.as_str(),
        cap,
    )
    .await?;
    Ok(redriven.len() as u32)
}

/// Run one embed-dispatch pass: (optionally re-drive dead jobs →) reap → claim up to `cap` embed jobs
/// → embed each resource → complete the clean ones. Returns a [`EmbedDispatchSummary`] for
/// cron/operator observability. `cap` defaults to [`DEFAULT_EMBED_DISPATCH_CAP`] when `None`.
///
/// When `redrive` is set, dead embed jobs are re-enqueued (up to `cap` resources) *before* the claim,
/// so a single operator call both revives and drains them. The per-minute cron passes `redrive = false`
/// — recovery stays operator-gated (`?redrive=true`) so a persistently-failing resource is observable
/// as `dead` rather than churning in a dead→redrive→dead loop.
///
/// A per-job embed error is caught and tallied as `failed` (the job stays `in_progress`; the reaper
/// retries it, then marks it `dead` at max attempts) — one bad resource never aborts the pass or the
/// other jobs. The pass never returns an error for an embed failure; it only surfaces `ApiError` for a
/// genuine queue/DB fault.
pub async fn dispatch_tick(
    pool: &PgPool,
    cap: Option<i32>,
    redrive: bool,
) -> ApiResult<EmbedDispatchSummary> {
    let persona = Persona::Embed.as_str();
    let dispatch = DispatchType::Embed.as_str();

    // Optional re-drive prelude: revive dead jobs so this same pass's claim can drain them.
    let redriven = if redrive {
        self::redrive(pool, cap).await?
    } else {
        0
    };

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
        redriven,
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

/// Derive each resource's [`EmbeddingStatus`] in one batch query (issue #299, Phase 4, design §8) —
/// no N+1 on the list/enrich path. For every id in `ids` it reads two facts: whether any current,
/// non-folded chunk still has a NULL embedding, and whether a live embed job exists. From those:
/// `ready` (nothing unembedded), else `pending` (unembedded + live job), else `failed` (unembedded, no
/// live job — dead or lost to a supersede race). An id with no chunks reads as `ready` (empty body is
/// trivially embedded). Ids absent from the returned map (should not happen — `unnest` yields one row
/// per id) are treated as `ready` by callers.
pub async fn embedding_status_batch(
    pool: &PgPool,
    ids: &[Uuid],
) -> ApiResult<HashMap<Uuid, EmbeddingStatus>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query!(
        r#"
        SELECT r.id AS "id!: Uuid",
               EXISTS (
                   SELECT 1
                     FROM kb_chunks ch
                     JOIN kb_content_blocks b ON b.id = ch.block_id
                    WHERE ch.resource_id = r.id
                      AND ch.is_current
                      AND NOT b.is_folded
                      AND ch.embedding IS NULL
               ) AS "unembedded!: bool",
               EXISTS (
                   SELECT 1
                     FROM kb_workflow_jobs j
                    WHERE j.resource_id = r.id
                      AND j.persona = 'embed'
                      AND j.dispatch_type = 'embed'
                      AND j.status IN ('pending', 'in_progress', 'waiting_for_retry')
               ) AS "live_job!: bool"
          FROM unnest($1::uuid[]) AS r(id)
        "#,
        ids,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let status = if !r.unembedded {
                EmbeddingStatus::Ready
            } else if r.live_job {
                EmbeddingStatus::Pending
            } else {
                EmbeddingStatus::Failed
            };
            (r.id, status)
        })
        .collect())
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

        let summary = dispatch_tick(&pool, Some(5), false).await.unwrap();
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
        let again = dispatch_tick(&pool, Some(5), false).await.unwrap();
        assert_eq!(again.claimed, 0);
    }

    // An empty queue is a clean no-op pass.
    #[sqlx::test(migrations = "../../migrations")]
    async fn dispatch_tick_empty_queue_is_noop(pool: PgPool) {
        let summary = dispatch_tick(&pool, None, false).await.unwrap();
        assert_eq!(summary.claimed, 0);
        assert_eq!(summary.completed, 0);
        assert_eq!(summary.chunks_embedded, 0);
    }

    // ── Phase 4: derived embedding_status + re-drive (all ONNX-free) ───────────────────────────
    //
    // These fixtures build the chunk graph by hand (a real migration DB seeds a few kb_events we can
    // borrow for the block's genesis/last event FKs), so the derivation and re-drive logic are
    // exercised without any ONNX inference. `embedding IS NULL` vs a filled 768-vector is all the
    // derivation reads; the vector's contents are irrelevant, so a zero-fill stands in.

    async fn a_named_resource(pool: &PgPool, title: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id",
        )
        .bind(title)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// A content block on `resource` at `seq`, reusing a migration-seeded event for the genesis/last
    /// FKs. `folded` blocks are excluded by the status derivation.
    async fn a_block(pool: &PgPool, resource: Uuid, seq: i32, folded: bool) -> Uuid {
        let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
            .fetch_one(pool)
            .await
            .unwrap();
        sqlx::query_scalar(
            "INSERT INTO kb_content_blocks (resource_id, seq, is_folded, genesis_event_id, last_event_id) \
             VALUES ($1, $2, $3, $4, $4) RETURNING id",
        )
        .bind(resource)
        .bind(seq)
        .bind(folded)
        .bind(ev)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// A chunk on `block`. `embedded` fills a zero 768-vector (any non-NULL vector reads as embedded);
    /// otherwise the embedding stays NULL (the deferred state).
    async fn a_chunk(
        pool: &PgPool,
        block: Uuid,
        resource: Uuid,
        idx: i32,
        embedded: bool,
        current: bool,
    ) {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, is_current) \
             VALUES ($1, $2, $3, 'h', $4) RETURNING id",
        )
        .bind(block)
        .bind(resource)
        .bind(idx)
        .bind(current)
        .fetch_one(pool)
        .await
        .unwrap();
        if embedded {
            sqlx::query("UPDATE kb_chunks SET embedding = array_fill(0::real, ARRAY[768])::vector WHERE id = $1")
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
        }
    }

    async fn set_job_status(pool: &PgPool, resource: Uuid, status: &str) {
        sqlx::query("UPDATE kb_workflow_jobs SET status = $2 WHERE resource_id = $1")
            .bind(resource)
            .bind(status)
            .execute(pool)
            .await
            .unwrap();
    }

    // The three derived states (design §8), plus the no-chunks and no-job edges, in one batch call.
    #[sqlx::test(migrations = "../../migrations")]
    async fn embedding_status_derives_ready_pending_failed(pool: PgPool) {
        // ready — a fully-embedded chunk, no job.
        let ready = a_named_resource(&pool, "ready").await;
        let b = a_block(&pool, ready, 0, false).await;
        a_chunk(&pool, b, ready, 0, true, true).await;

        // ready — a resource with no chunks at all (empty body is trivially embedded).
        let empty = a_named_resource(&pool, "empty").await;

        // pending — a NULL-embedding chunk with a live (pending) embed job.
        let pending = a_named_resource(&pool, "pending").await;
        let pb = a_block(&pool, pending, 0, false).await;
        a_chunk(&pool, pb, pending, 0, false, true).await;
        workflow_job_service::enqueue_resource(&pool, pending, "embed", "embed")
            .await
            .unwrap()
            .expect("enqueue");

        // failed — a NULL-embedding chunk whose only embed job is dead.
        let failed = a_named_resource(&pool, "failed").await;
        let fb = a_block(&pool, failed, 0, false).await;
        a_chunk(&pool, fb, failed, 0, false, true).await;
        workflow_job_service::enqueue_resource(&pool, failed, "embed", "embed")
            .await
            .unwrap()
            .expect("enqueue");
        set_job_status(&pool, failed, "dead").await;

        // failed — a NULL-embedding chunk with no job at all (supersede-race shape).
        let orphan = a_named_resource(&pool, "orphan").await;
        let ob = a_block(&pool, orphan, 0, false).await;
        a_chunk(&pool, ob, orphan, 0, false, true).await;

        let ids = [ready, empty, pending, failed, orphan];
        let got = embedding_status_batch(&pool, &ids).await.unwrap();
        assert_eq!(got[&ready], EmbeddingStatus::Ready);
        assert_eq!(got[&empty], EmbeddingStatus::Ready, "no chunks ⇒ ready");
        assert_eq!(got[&pending], EmbeddingStatus::Pending);
        assert_eq!(got[&failed], EmbeddingStatus::Failed, "dead job ⇒ failed");
        assert_eq!(
            got[&orphan],
            EmbeddingStatus::Failed,
            "no live job ⇒ failed"
        );
    }

    // The derivation ignores non-current and folded-block chunks: a resource whose ONLY NULL-embedding
    // chunk is superseded (not current) or on a folded block reads as `ready`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn embedding_status_ignores_noncurrent_and_folded(pool: PgPool) {
        // Only NULL chunk is non-current (an old generation) — the current one is embedded.
        let superseded = a_named_resource(&pool, "superseded").await;
        let sb = a_block(&pool, superseded, 0, false).await;
        a_chunk(&pool, sb, superseded, 0, false, false).await; // old, NULL, not current
        a_chunk(&pool, sb, superseded, 1, true, true).await; // current, embedded

        // Only NULL chunk lives on a folded block.
        let folded = a_named_resource(&pool, "folded").await;
        let foldb = a_block(&pool, folded, 0, true).await;
        a_chunk(&pool, foldb, folded, 0, false, true).await;

        let got = embedding_status_batch(&pool, &[superseded, folded])
            .await
            .unwrap();
        assert_eq!(
            got[&superseded],
            EmbeddingStatus::Ready,
            "non-current NULL chunk is ignored"
        );
        assert_eq!(
            got[&folded],
            EmbeddingStatus::Ready,
            "folded-block NULL chunk is ignored"
        );
    }

    // Re-drive resurrects a dead embed job as a fresh pending one; a resource with a live job is left
    // alone. ONNX-free: the resources are chunkless, so nothing is inferred here.
    #[sqlx::test(migrations = "../../migrations")]
    async fn redrive_reenqueues_dead_only(pool: PgPool) {
        // A dead-jobbed resource.
        let dead = a_resource(&pool).await;
        workflow_job_service::enqueue_resource(&pool, dead, "embed", "embed")
            .await
            .unwrap()
            .expect("enqueue");
        set_job_status(&pool, dead, "dead").await;

        // A resource whose job is still live — re-drive must not touch it.
        let live = a_named_resource(&pool, "live").await;
        workflow_job_service::enqueue_resource(&pool, live, "embed", "embed")
            .await
            .unwrap()
            .expect("enqueue");

        let n = redrive(&pool, None).await.unwrap();
        assert_eq!(n, 1, "only the dead-jobbed resource is re-driven");

        // The dead resource now has a fresh pending job (the dead row stays as history).
        let pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_workflow_jobs WHERE resource_id = $1 AND status = 'pending'",
        )
        .bind(dead)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pending, 1, "dead job re-enqueued as pending");

        // Re-drive is idempotent while a live job exists: a second pass re-drives nothing.
        assert_eq!(redrive(&pool, None).await.unwrap(), 0);
    }

    // The `?redrive=true` composition: a single dispatch pass revives a dead job and drains it in the
    // same tick. Chunkless resource ⇒ the embed is a clean no-op, so this stays ONNX-free.
    #[sqlx::test(migrations = "../../migrations")]
    async fn dispatch_tick_with_redrive_revives_and_drains(pool: PgPool) {
        let r = a_resource(&pool).await;
        workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
            .await
            .unwrap()
            .expect("enqueue");
        set_job_status(&pool, r, "dead").await;

        // Without re-drive, a plain tick finds nothing claimable (the job is dead, not pending).
        let plain = dispatch_tick(&pool, Some(5), false).await.unwrap();
        assert_eq!(plain.redriven, 0);
        assert_eq!(plain.claimed, 0, "a dead job is not claimable");

        // With re-drive, the same pass re-enqueues the dead job and then claims + completes it.
        let summary = dispatch_tick(&pool, Some(5), true).await.unwrap();
        assert_eq!(summary.redriven, 1);
        assert_eq!(
            summary.claimed, 1,
            "the re-driven job is drained in the same pass"
        );
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.failed, 0);
    }
}
