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

use crate::error::{ApiError, ApiResult};
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

/// Warm the ONNX embedder — cold-start mitigation for server-side query embedding (issue #427).
///
/// A search whose caller cannot precompute an embedding (MCP, the web UI, `--text-only`) is embedded
/// server-side *inside the request* (`substrate_read::embed_query_if_missing`). On a COLD serverless
/// instance that pays a one-time model load (ORT init + writing the bundled runtime to `/tmp` +
/// reading the quantized model + first inference); run inside the request that cost can exceed the
/// query-embed budget (`TEMPER_QUERY_EMBED_BUDGET_MS`, default 8s), and the vector arm is silently
/// dropped to FTS + graph. On a low-traffic deploy where the function scales to zero between searches,
/// *every* search pays it — which is exactly the "vector ranking was unavailable" report this fixes.
///
/// This populates the process's model cache (`MODEL: OnceLock`) BEFORE a real search arrives, so the
/// embed inside the request is a cheap cached inference. Called by the cron-driven `/api/embed/warm`
/// endpoint to keep the serving instance hot.
///
/// The embed is CPU-bound and blocking, so it runs off the async executor via `spawn_blocking` —
/// exactly as the search path does. Returns the embedding dimensionality as a liveness signal (768 for
/// bge-base); the vector itself is discarded.
///
/// NOTE: this is a *best-effort* warmth, not a guarantee — Vercel does not promise the cron and a user
/// request land on the same instance. The durable "cold embed fits the budget" guarantee is the
/// function's memory (and thus CPU) in `vercel.json`; this reduces how often a cold embed happens at
/// all.
pub async fn warm_embedder() -> ApiResult<usize> {
    let dims = tokio::task::spawn_blocking(|| temper_ingest::embed::embed_text("warm"))
        .await
        .map_err(|e| ApiError::Internal(format!("embed warm task panicked: {e}")))?
        .map_err(|e| ApiError::Internal(format!("embed warm failed: {e}")))?
        .len();
    Ok(dims)
}

/// Default per-invocation wall-clock lifetime for one loop-draining dispatch pass, in seconds.
///
/// Under loop-drain this is the **invocation lifetime**, not a single-pass guard: `dispatch_tick`
/// keeps claiming until the queue is empty or this deadline is hit. Set to **55s** — just under the
/// one-minute cron cadence — so each fire finishes before the next fires. Back-to-back invocations
/// then tile the timeline with no stacking, making effective concurrency equal to the number of
/// shard cron lines (the "predictable N" model, spec Option A): concurrency = N, cost linear in N,
/// Neon connections bounded at ~N x 2.
///
/// `maxDuration` (300s in `vercel.json`) stays a pure safety ceiling far above this. To trade
/// predictable concurrency for higher per-shard throughput (spec Option B), raise
/// `TEMPER_EMBED_DISPATCH_DEADLINE_SECONDS` toward ~250 on a specific deploy: invocations then run
/// near `maxDuration` and a minute-cadence cron stacks ~4 per shard (effective ~4N). No code change.
pub const DEFAULT_EMBED_DISPATCH_DEADLINE_SECONDS: u64 = 55;

/// Env override for the per-pass wall-clock ceiling, in seconds. A malformed or zero value falls back
/// to the default rather than silently disabling the bound — mirroring `resolve_chunk_budget`. Retune
/// alongside `maxDuration` and `TEMPER_EMBED_CHUNK_BUDGET` once real ms/chunk is measured.
pub const EMBED_DISPATCH_DEADLINE_ENV: &str = "TEMPER_EMBED_DISPATCH_DEADLINE_SECONDS";

/// Resolve the per-pass wall-clock ceiling as a [`std::time::Duration`].
pub fn resolve_dispatch_deadline() -> std::time::Duration {
    let secs = std::env::var(EMBED_DISPATCH_DEADLINE_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_EMBED_DISPATCH_DEADLINE_SECONDS);
    std::time::Duration::from_secs(secs)
}

/// What slice of the index to re-embed. Deliberately three granularities, because a re-embed is a
/// thing you want to try on **one** resource, then a **handful**, then **everything** — in that order.
/// A mechanism that only offers "all" is one nobody dares run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReembedScope {
    /// Exactly one resource.
    Resource(Uuid),
    /// Every resource in one context.
    Context(Uuid),
    /// The whole index.
    All,
}

/// Enqueue embed jobs for resources holding **stale** chunks in `scope`, up to `limit` resources.
///
/// This is the *trigger* for the re-embed; the drain (`dispatch_tick`, running every minute) is the
/// engine. Staleness is not a thing we mark — it is a thing we *observe*: a chunk is stale when its
/// `embedded_with` is not the model this build embeds with. So there is no "mark dirty" write, no
/// backfill migration, and no window in which a chunk is unsearchable. Rows written before provenance
/// existed are stale by definition, and any future model change re-stales the index automatically.
///
/// **Nothing calls this on a schedule.** The drain consumes jobs; it does not go looking for stale
/// chunks. Re-embedding only ever begins because an operator asked for it — which is the point: you
/// aim it at one resource before you aim it at 31,824. The corollary is that a deploy heals nothing on
/// its own, and any write from an un-upgraded client keeps adding unstamped vectors that will sit
/// stale until the next sweep.
///
/// Returns the resource ids enqueued. Resources with a live job are skipped (the underlying
/// `ON CONFLICT DO NOTHING`), so this is safe to run repeatedly and safe to run while the drain is
/// mid-flight — re-running it simply picks up whatever is still stale.
pub async fn enqueue_stale(pool: &PgPool, scope: ReembedScope, limit: i32) -> ApiResult<Vec<Uuid>> {
    let model = temper_ingest::embed::EXPECTED_MODEL_SHA256;

    // One predicate, three scopes. The `$2::uuid IS NULL OR ...` shape keeps this a single prepared
    // statement rather than three near-identical ones that can drift apart.
    //
    // A resource has no context COLUMN — it is *homed* in one via `kb_resource_homes`
    // (anchor_table = 'kb_contexts'). The same table also homes cogmap nodes
    // (anchor_table = 'kb_cogmaps'), so the anchor_table predicate is load-bearing, not decoration:
    // without it, a context-scoped re-embed would sweep in cogmap-homed resources too.
    let (resource_filter, context_filter) = match scope {
        ReembedScope::Resource(id) => (Some(id), None),
        ReembedScope::Context(id) => (None, Some(id)),
        ReembedScope::All => (None, None),
    };

    let stale: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT r.id \
           FROM kb_resources r \
           JOIN kb_chunks ch ON ch.resource_id = r.id \
           JOIN kb_content_blocks b ON b.id = ch.block_id \
          WHERE r.is_active \
            AND ch.is_current \
            AND NOT b.is_folded \
            AND ch.embedded_with IS DISTINCT FROM $1 \
            AND ($2::uuid IS NULL OR r.id = $2::uuid) \
            AND ($3::uuid IS NULL OR EXISTS ( \
                    SELECT 1 FROM kb_resource_homes h \
                     WHERE h.resource_id = r.id \
                       AND h.anchor_table = 'kb_contexts' \
                       AND h.anchor_id = $3::uuid)) \
          LIMIT $4",
    )
    .bind(model)
    .bind(resource_filter)
    .bind(context_filter)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;

    // The (persona, dispatch_type) tuple MUST be the one `dispatch_tick` claims on — a job enqueued
    // under any other tuple is invisible to the drain and would sit pending forever, looking enqueued
    // and doing nothing.
    let persona = Persona::Embed.as_str();
    let dispatch = DispatchType::Embed.as_str();

    let mut enqueued = Vec::with_capacity(stale.len());
    for resource_id in stale {
        if workflow_job_service::enqueue_resource(pool, resource_id, persona, dispatch)
            .await?
            .is_some()
        {
            enqueued.push(resource_id);
        }
    }
    Ok(enqueued)
}

/// How many resources in `scope` still hold stale chunks, and how many stale chunks in total.
///
/// The honest progress readout for a re-embed: "is this converging?" answered without guessing.
pub async fn stale_summary(pool: &PgPool, scope: ReembedScope) -> ApiResult<(u64, u64)> {
    let model = temper_ingest::embed::EXPECTED_MODEL_SHA256;
    let (resource_filter, context_filter) = match scope {
        ReembedScope::Resource(id) => (Some(id), None),
        ReembedScope::Context(id) => (None, Some(id)),
        ReembedScope::All => (None, None),
    };

    let row: (i64, i64) = sqlx::query_as(
        "SELECT count(DISTINCT r.id), count(*) \
           FROM kb_resources r \
           JOIN kb_chunks ch ON ch.resource_id = r.id \
           JOIN kb_content_blocks b ON b.id = ch.block_id \
          WHERE r.is_active \
            AND ch.is_current \
            AND NOT b.is_folded \
            AND ch.embedded_with IS DISTINCT FROM $1 \
            AND ($2::uuid IS NULL OR r.id = $2::uuid) \
            AND ($3::uuid IS NULL OR EXISTS ( \
                    SELECT 1 FROM kb_resource_homes h \
                     WHERE h.resource_id = r.id \
                       AND h.anchor_table = 'kb_contexts' \
                       AND h.anchor_id = $3::uuid))",
    )
    .bind(model)
    .bind(resource_filter)
    .bind(context_filter)
    .fetch_one(pool)
    .await?;

    Ok((row.0 as u64, row.1 as u64))
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

/// Run one embed-dispatch invocation: (optionally re-drive dead jobs →) reap once → then **loop**
/// {claim up to `cap` embed jobs → embed each resource → complete the clean ones, re-enqueue the
/// rest} until the queue is empty or the wall-clock deadline is hit. Returns a
/// [`EmbedDispatchSummary`] (whose counts span the whole loop, not one claim) for cron/operator
/// observability. `cap` defaults to [`DEFAULT_EMBED_DISPATCH_CAP`] when `None`.
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
    dispatch_tick_inner(pool, cap, redrive, resolve_dispatch_deadline()).await
}

/// [`dispatch_tick`] with the wall-clock ceiling injected, so tests can force the deadline-defer path
/// deterministically (a `Duration::ZERO` defers every claimed job on its first check) without sleeping.
async fn dispatch_tick_inner(
    pool: &PgPool,
    cap: Option<i32>,
    redrive: bool,
    deadline: std::time::Duration,
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

    let mut summary = EmbedDispatchSummary {
        redriven,
        ..Default::default()
    };

    // Hard wall-clock ceiling on the whole invocation (see `resolve_dispatch_deadline`). With
    // loop-drain the deadline is the *invocation lifetime*, not a single-pass guard: we keep
    // claiming until the queue is empty or the deadline is hit, so one invocation fills its
    // wall-clock instead of returning after a single ~64-chunk claim.
    let start = std::time::Instant::now();

    // Do-while shape: always run at least one claim, then stop once past the deadline. Checking the
    // deadline AFTER a full claim (not before the first) guarantees every invocation runs at least
    // one claim attempt and can never spin, even under a pathologically small deadline — and preserves
    // the ZERO-deadline defer semantics
    // the wall-clock test asserts (claim once, defer the batch, break).
    loop {
        let claimed = workflow_job_service::claim_resource(
            pool,
            persona,
            dispatch,
            cap,
            DEFAULT_EMBED_LEASE_SECONDS,
        )
        .await?;
        if claimed.is_empty() {
            break; // queue drained — nothing left to do this invocation
        }
        summary.claimed += claimed.len() as u32;

        // ONE chunk allowance per CLAIM (see `EMBED_CHUNK_BUDGET`), refreshed each iteration. The
        // deadline bounds the invocation; this bounds one claim's inference so no single
        // `embed_texts` call is ever large — which is what retires the single-large-resource cliff:
        // a 939-chunk resource embeds 64, re-enqueues, and is simply re-claimed on a later iteration.
        let mut budget = temper_substrate::embed::resolve_chunk_budget();

        for job in claimed {
            if start.elapsed() >= deadline {
                // Past the deadline: defer this job (and every one after it) — re-enqueue untouched
                // to resume next tick rather than hold a lease (a held lease looks like a crash to
                // the reaper and is not reclaimable for DEFAULT_EMBED_LEASE_SECONDS). Tallied
                // `partial`, not `failed` — 0 chunks embedded, job resumed later.
                workflow_job_service::complete_resource(pool, job.resource_id, persona, dispatch)
                    .await?;
                workflow_job_service::enqueue_resource(pool, job.resource_id, persona, dispatch)
                    .await?;
                tracing::info!(
                    resource_id = %job.resource_id,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    "embed dispatch hit its wall-clock deadline; re-enqueued job for the next tick"
                );
                summary.partial += 1;
                continue;
            }
            match temper_substrate::embed::embed_resource_chunks(pool, job.resource_id, budget)
                .await
            {
                Ok(progress) => {
                    summary.chunks_embedded += progress.embedded;
                    budget -= progress.embedded as i64;
                    if progress.is_complete() {
                        workflow_job_service::complete_resource(
                            pool,
                            job.resource_id,
                            persona,
                            dispatch,
                        )
                        .await?;
                        summary.completed += 1;
                    } else {
                        // More stale chunks than this claim's budget: complete + re-enqueue so a
                        // later iteration (this invocation, or the next tick) resumes it with a
                        // fresh budget. Complete-then-enqueue (not hold the lease) keeps the reaper's
                        // attempt count clean for large resources. The two writes are NOT atomic: a
                        // crash between them strands the resource with no job — and there is no
                        // automatic stale sweep (`enqueue_stale` is operator-triggered), so recovery
                        // is the next operator `temper admin reembed` over a scope containing it.
                        workflow_job_service::complete_resource(
                            pool,
                            job.resource_id,
                            persona,
                            dispatch,
                        )
                        .await?;
                        workflow_job_service::enqueue_resource(
                            pool,
                            job.resource_id,
                            persona,
                            dispatch,
                        )
                        .await?;
                        tracing::info!(
                            resource_id = %job.resource_id,
                            embedded = progress.embedded,
                            remaining = progress.remaining,
                            "embed job partially drained; re-enqueued for the next tick"
                        );
                        summary.partial += 1;
                    }
                }
                Err(e) => {
                    // Leave the job in_progress; the reaper's lease-expiry sweep retries it (then
                    // dead at max attempts). One bad resource never aborts the pass.
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

        // Stop looping once past the deadline — checked after a full claim, so ≥1 claim always runs.
        if start.elapsed() >= deadline {
            break;
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

    // The wall-clock guard: a pass that is already past its deadline embeds nothing and re-enqueues the
    // claimed job untouched (tallied `partial`, not `failed`), so the next tick resumes it. A
    // `Duration::ZERO` deadline forces the defer branch on the first check without sleeping. ONNX-free:
    // the resource is chunkless, and the defer path never calls the embedder anyway.
    #[sqlx::test(migrations = "../../migrations")]
    async fn dispatch_tick_defers_jobs_past_the_wall_clock_deadline(pool: PgPool) {
        let r = a_resource(&pool).await;
        workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
            .await
            .unwrap()
            .expect("enqueue");

        let summary = dispatch_tick_inner(&pool, Some(5), false, std::time::Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(summary.claimed, 1);
        assert_eq!(
            summary.partial, 1,
            "a deadline-deferred job is partial (re-enqueued), not failed"
        );
        assert_eq!(summary.completed, 0);
        assert_eq!(summary.failed, 0);
        assert_eq!(
            summary.chunks_embedded, 0,
            "nothing is embedded past the deadline"
        );

        // The deferred job was completed + re-enqueued, so the resource holds one fresh pending job.
        let pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_workflow_jobs WHERE resource_id = $1 AND status = 'pending'",
        )
        .bind(r)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pending, 1, "deferred job re-enqueued as pending");

        // A normal-deadline tick then drains the re-enqueued job cleanly — progress is monotonic.
        let drained = dispatch_tick(&pool, Some(5), false).await.unwrap();
        assert_eq!(drained.claimed, 1);
        assert_eq!(
            drained.completed, 1,
            "the re-enqueued job drains on the next tick"
        );
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

    async fn stamp(pool: &PgPool, resource: Uuid, sha: &str) {
        sqlx::query("UPDATE kb_chunks SET embedded_with = $2 WHERE resource_id = $1")
            .bind(resource)
            .bind(sha)
            .execute(pool)
            .await
            .unwrap();
    }

    /// **The regression test for the model split.** A chunk that HAS a vector but no provenance is
    /// exactly the shape of every row written before `embedded_with` existed — i.e. ~95% of the real
    /// index, all of it fp32. Under the old predicate (`embedding IS NULL`) such a chunk was considered
    /// perfectly fine and would never have been re-embedded. It must now read as stale.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_embedded_chunk_with_unknown_provenance_is_stale(pool: PgPool) {
        let r = a_named_resource(&pool, "legacy").await;
        let b = a_block(&pool, r, 0, false).await;
        a_chunk(&pool, b, r, 0, true, true).await; // has a vector, no embedded_with

        let (resources, chunks) = stale_summary(&pool, ReembedScope::All).await.unwrap();
        assert_eq!(
            (resources, chunks),
            (1, 1),
            "an embedded-but-unstamped chunk must be STALE — it is the fp32 case"
        );
    }

    /// The other half: a chunk stamped with the model this build actually embeds with is NOT stale, so
    /// the drain converges instead of re-embedding the same rows forever.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_chunk_stamped_with_the_current_model_is_not_stale(pool: PgPool) {
        let r = a_named_resource(&pool, "fresh").await;
        let b = a_block(&pool, r, 0, false).await;
        a_chunk(&pool, b, r, 0, true, true).await;
        stamp(&pool, r, temper_ingest::embed::EXPECTED_MODEL_SHA256).await;

        let (resources, chunks) = stale_summary(&pool, ReembedScope::All).await.unwrap();
        assert_eq!(
            (resources, chunks),
            (0, 0),
            "current-model vectors are fresh"
        );
    }

    /// A vector stamped by SOME OTHER model is stale — this is what makes a future model change
    /// a future model change need no migration: the constant changes, the whole index re-stales itself,
    /// and an operator sweep drains it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_chunk_stamped_with_a_different_model_is_stale(pool: PgPool) {
        let r = a_named_resource(&pool, "other-model").await;
        let b = a_block(&pool, r, 0, false).await;
        a_chunk(&pool, b, r, 0, true, true).await;
        stamp(
            &pool,
            r,
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .await;

        let (resources, _) = stale_summary(&pool, ReembedScope::All).await.unwrap();
        assert_eq!(resources, 1, "a foreign model's vectors are stale");
    }

    /// Scoping is the whole point of the trigger: try ONE resource before you try 31,000.
    #[sqlx::test(migrations = "../../migrations")]
    async fn enqueue_stale_scopes_to_a_single_resource(pool: PgPool) {
        let a = a_named_resource(&pool, "a").await;
        let ba = a_block(&pool, a, 0, false).await;
        a_chunk(&pool, ba, a, 0, true, true).await;

        let b = a_named_resource(&pool, "b").await;
        let bb = a_block(&pool, b, 0, false).await;
        a_chunk(&pool, bb, b, 0, true, true).await;

        let (all_stale, _) = stale_summary(&pool, ReembedScope::All).await.unwrap();
        assert_eq!(all_stale, 2, "both resources are stale");

        let enqueued = enqueue_stale(&pool, ReembedScope::Resource(a), 100)
            .await
            .unwrap();
        assert_eq!(enqueued, vec![a], "only the scoped resource is enqueued");

        // Idempotent: the resource now has a live job, so a second call adds nothing.
        let again = enqueue_stale(&pool, ReembedScope::Resource(a), 100)
            .await
            .unwrap();
        assert!(
            again.is_empty(),
            "a resource with a live job is not enqueued twice"
        );
    }

    /// Context scoping goes through `kb_resource_homes` (a resource has no context column), and the
    /// `anchor_table = 'kb_contexts'` predicate is load-bearing: the SAME table homes cogmap nodes, so
    /// without it a context-scoped re-embed would silently drag in every cogmap-homed resource too.
    #[sqlx::test(migrations = "../../migrations")]
    async fn enqueue_stale_scopes_to_a_context_and_excludes_cogmap_homes(pool: PgPool) {
        let owner: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ('scope-owner', 'Scope Owner') \
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let ctx: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, name, slug) \
             VALUES ('kb_profiles', $1, 'scope-ctx', 'scope-ctx') RETURNING id",
        )
        .bind(owner)
        .fetch_one(&pool)
        .await
        .unwrap();

        // In the context.
        let inside = a_named_resource(&pool, "inside").await;
        let bi = a_block(&pool, inside, 0, false).await;
        a_chunk(&pool, bi, inside, 0, true, true).await;
        sqlx::query(
            "INSERT INTO kb_resource_homes \
                 (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_contexts', $2, $3, $3)",
        )
        .bind(inside)
        .bind(ctx)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();

        // Homed in a COGMAP whose id happens to equal the context id — the exact collision the
        // anchor_table predicate exists to reject. Without it this resource would be swept in.
        let decoy = a_named_resource(&pool, "cogmap-homed").await;
        let bd = a_block(&pool, decoy, 0, false).await;
        a_chunk(&pool, bd, decoy, 0, true, true).await;
        sqlx::query(
            "INSERT INTO kb_resource_homes \
                 (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
        )
        .bind(decoy)
        .bind(ctx)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();

        let (stale, _) = stale_summary(&pool, ReembedScope::Context(ctx))
            .await
            .unwrap();
        assert_eq!(stale, 1, "only the context-homed resource is in scope");

        let enqueued = enqueue_stale(&pool, ReembedScope::Context(ctx), 100)
            .await
            .unwrap();
        assert_eq!(
            enqueued,
            vec![inside],
            "a cogmap-homed resource must NOT be swept into a context re-embed"
        );
    }

    /// **The wedge test.** A blank chunk has nothing to embed. It must still CONVERGE — i.e. stop
    /// being stale — or `remaining` never reaches zero, the drain re-enqueues that resource every
    /// minute forever embedding nothing, and `stale_summary` never converges (destroying the
    /// operator's only progress signal).
    ///
    /// This is why STALE_CHUNK_PREDICATE keys on `embedded_with` ALONE. With the redundant
    /// `embedding IS NULL OR` disjunct it originally carried, stamping a blank chunk could never clear
    /// it and this test would hang the drain in perpetuity.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_blank_chunk_converges_instead_of_wedging_the_drain(pool: PgPool) {
        let r = a_named_resource(&pool, "blank").await;
        let b = a_block(&pool, r, 0, false).await;

        let chunk: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, is_current) \
             VALUES ($1, $2, 0, 'h', true) RETURNING id",
        )
        .bind(b)
        .bind(r)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1, '   ')")
            .bind(chunk)
            .execute(&pool)
            .await
            .unwrap();

        let progress = temper_substrate::embed::embed_resource_chunks(
            &pool,
            r,
            temper_substrate::embed::EMBED_CHUNK_BUDGET,
        )
        .await
        .unwrap();
        assert_eq!(progress.embedded, 0, "a blank chunk embeds nothing");
        assert!(
            progress.is_complete(),
            "...but it MUST converge: remaining={} — a non-zero remaining here means the drain \
             re-enqueues this resource every minute forever",
            progress.remaining
        );

        let (stale, _) = stale_summary(&pool, ReembedScope::All).await.unwrap();
        assert_eq!(
            stale, 0,
            "and the operator's convergence signal reaches zero"
        );
    }

    /// The enqueued job must be claimable by the drain — i.e. enqueued under the SAME
    /// (persona, dispatch_type) tuple `dispatch_tick` claims on. Get this wrong and the job sits
    /// pending forever, looking enqueued and doing nothing.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_stale_enqueued_resource_is_claimed_by_the_drain(pool: PgPool) {
        let r = a_named_resource(&pool, "drainable").await;
        let b = a_block(&pool, r, 0, false).await;
        a_chunk(&pool, b, r, 0, true, true).await;

        let enqueued = enqueue_stale(&pool, ReembedScope::All, 100).await.unwrap();
        assert_eq!(enqueued, vec![r]);

        let summary = dispatch_tick(&pool, Some(5), false).await.unwrap();
        assert_eq!(
            summary.claimed, 1,
            "the drain must claim what enqueue_stale queued"
        );
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

    /// **The loop-drain gate.** Three enqueued resources with cap=1 → one resource per claim. A
    /// single-claim pass (the old behavior) would drain exactly ONE and return; loop-drain must drain
    /// all THREE inside one invocation by re-claiming until the queue is empty. Chunkless resources
    /// keep this ONNX-free (embed is a clean no-op).
    #[sqlx::test(migrations = "../../migrations")]
    async fn dispatch_tick_loop_drains_multiple_claims_in_one_pass(pool: PgPool) {
        for name in ["a", "b", "c"] {
            let r = a_named_resource(&pool, name).await;
            workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
                .await
                .unwrap()
                .expect("enqueue");
        }

        let summary = dispatch_tick(&pool, Some(1), false).await.unwrap();
        assert_eq!(
            summary.claimed, 3,
            "loop-drain re-claims until the queue is empty — not just one claim"
        );
        assert_eq!(summary.completed, 3);
        assert_eq!(summary.failed, 0);

        // Idempotent: a second pass finds nothing.
        let again = dispatch_tick(&pool, Some(1), false).await.unwrap();
        assert_eq!(again.claimed, 0);
    }
}
