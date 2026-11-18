//! The async-embedding drain (issue #299). One `dispatch_tick` pass reaps stale leases, claims a
//! batch of resource-keyed embed jobs, backfills each resource's deferred chunk embeddings
//! (`temper_substrate::embed::embed_resource_chunks`), and marks the clean ones done. A failed embed
//! is left `in_progress` for the reaper's retry→dead path — errors are off the create request path.
//!
//! SERVICE-DIRECT (not a `DbBackend` command): embedding is a **system** backfill with no profile
//! scoping — unlike the steward dispatch, which sweeps profile-visible maps. The `/api/embed/dispatch`
//! endpoint (CRON_SECRET-gated) is the only caller; there is no per-resource authz because the queue
//! is fed only by the trusted server-side write path.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::SystemAdmin;
use crate::error::{ApiError, ApiResult};
use crate::services::drain_span::{self, JobOutcome};
use crate::services::workflow_job_service;
use temper_core::types::workflow_job::{
    ClaimedEmbedJob, DispatchType, EmbedDispatchSummary, EmbeddingStatus, Persona,
    DEFAULT_EMBED_DISPATCH_CAP, DEFAULT_EMBED_LEASE_SECONDS,
};
use temper_substrate::embed::ERASED_HASH_EXCLUSION;

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

/// Whether a re-embed scope aims at someone else's personal space — one resource homed in, or one
/// context that is, a profile's or a personal team's other than the operator's. A system admin
/// does not aim at another's personal space (task `01a191b4`); the whole-index sweep still
/// re-derives its embeddings, so its owner's search keeps working, but does not name it (see
/// [`unnamed_personal`]).
pub async fn scope_is_personal(
    pool: &PgPool,
    admin: &SystemAdmin,
    scope: ReembedScope,
) -> ApiResult<bool> {
    let operator = admin.actor();
    Ok(match scope {
        ReembedScope::Resource(id) => {
            sqlx::query_scalar!(
                r#"SELECT COALESCE(resource_personal_owner($1) <> $2, false) AS "p!: bool""#,
                id,
                *operator,
            )
            .fetch_one(pool)
            .await?
        }
        ReembedScope::Context(id) => {
            crate::services::context_service::is_others_personal_context(pool, id, operator).await?
        }
        ReembedScope::All => false,
    })
}

/// The refusal for a re-embed aimed at someone else's personal space, or `None` when it is not. The
/// same sentence every door gives an admin stopped at personal space (task `01a1933d`): it names
/// the kind and the person, never what is stale there.
pub async fn personal_scope_refusal(
    pool: &PgPool,
    admin: &SystemAdmin,
    scope: ReembedScope,
) -> ApiResult<Option<ApiError>> {
    let subject = match scope {
        ReembedScope::Resource(id) => temper_substrate::payloads::RefTarget {
            kind: temper_substrate::payloads::AnchorTable::Resources,
            id,
        },
        ReembedScope::Context(id) => temper_substrate::payloads::RefTarget {
            kind: temper_substrate::payloads::AnchorTable::Contexts,
            id,
        },
        ReembedScope::All => return Ok(None),
    };
    match crate::authz::personal_space::others_personal_space(pool, subject, admin.actor()).await? {
        Some(stop) => Ok(Some(
            crate::authz::personal_space::refusal(pool, stop).await?,
        )),
        None => Ok(None),
    }
}

/// The ids of `enqueued` that are not homed in someone else's personal space, in order. The sweep
/// enqueues personal resources like any others; the operator's receipt names only the rest (task
/// `01a191b4`).
pub async fn unnamed_personal(
    pool: &PgPool,
    admin: &SystemAdmin,
    enqueued: Vec<Uuid>,
) -> ApiResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM unnest($1::uuid[]) WITH ORDINALITY AS u(id, n)
            WHERE COALESCE(resource_personal_owner(id) = $2, true) ORDER BY n"#,
        &enqueued,
        *admin.actor(),
    )
    .fetch_all(pool)
    .await?)
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
///
/// Operator-only: enqueuing index-wide re-embed work is a system-authority act, so it takes a
/// `&SystemAdmin` proof (admin-authz enclosure, spec §3.2). The `_admin` param is the capability —
/// the automatic stale sweep does not exist; every enqueue is operator-triggered. Its ungated sibling
/// `stale_summary` is a pure read (the dry-run path uses it), so the proof stays off it.
pub async fn enqueue_stale(
    pool: &PgPool,
    _admin: &SystemAdmin,
    scope: ReembedScope,
    limit: i32,
) -> ApiResult<Vec<Uuid>> {
    let model = temper_ingest::embed::EXPECTED_MODEL_SHA256;

    // One predicate, three scopes. The `$2::uuid IS NULL OR ...` shape keeps this a single prepared
    // statement rather than three near-identical ones that can drift apart.
    //
    // A resource has no context COLUMN — it is *homed* in one via `kb_resource_homes`
    // (anchor_table = 'kb_contexts'). The same table also homes cogmap nodes
    // (anchor_table = 'kb_cogmaps'), so the anchor_table predicate is load-bearing, not decoration:
    // without it, a context-scoped re-embed would sweep in cogmap-homed resources too.
    //
    // The erased-content exclusion is ERASED_HASH_EXCLUSION, interpolated (D4 arm 2 — see the
    // const for why exclusion, not loop-skip, and why this query is runtime SQL rather than a
    // `query!` macro: the shared clause cannot ride a compile-time-checked string).
    let (resource_filter, context_filter) = match scope {
        ReembedScope::Resource(id) => (Some(id), None),
        ReembedScope::Context(id) => (None, Some(id)),
        ReembedScope::All => (None, None),
    };

    let stale: Vec<Uuid> = sqlx::query_scalar(&format!(
        "SELECT DISTINCT r.id \
            FROM kb_resources r \
            JOIN kb_chunks ch ON ch.resource_id = r.id \
            JOIN kb_content_blocks b ON b.id = ch.block_id \
           WHERE r.is_active \
             AND ch.is_current \
             AND NOT b.is_folded \
             AND ch.embedded_with IS DISTINCT FROM $1 \
             AND {ERASED_HASH_EXCLUSION} \
             AND ($2::uuid IS NULL OR r.id = $2::uuid) \
             AND ($3::uuid IS NULL OR EXISTS ( \
                     SELECT 1 FROM kb_resource_homes h \
                      WHERE h.resource_id = r.id \
                        AND h.anchor_table = 'kb_contexts' \
                        AND h.anchor_id = $3::uuid)) \
           LIMIT $4"
    ))
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

    // Both counts take their columns positionally: `count()` over an empty grouping set still
    // returns 0 — it is never NULL.
    //
    // The erased-content exclusion is ERASED_HASH_EXCLUSION, interpolated (D4 arm 2, re-keyed
    // to the row by 20260911000000): the operator's convergence readout must count what the
    // drain may lawfully embed — a WIPED chunk (own content empty + set membership) is never
    // that, and counting it would read as a wedge that cannot converge; a same-hash chunk
    // with live prose IS work and counts. Same runtime-SQL reason as `enqueue_stale` above.
    let (resources, chunks): (i64, i64) = sqlx::query_as(&format!(
        r#"SELECT count(DISTINCT r.id), count(*)
            FROM kb_resources r
            JOIN kb_chunks ch ON ch.resource_id = r.id
            JOIN kb_content_blocks b ON b.id = ch.block_id
           WHERE r.is_active
             AND ch.is_current
             AND NOT b.is_folded
             AND ch.embedded_with IS DISTINCT FROM $1
             AND {ERASED_HASH_EXCLUSION}
             AND ($2::uuid IS NULL OR r.id = $2::uuid)
             AND ($3::uuid IS NULL OR EXISTS (
                     SELECT 1 FROM kb_resource_homes h
                      WHERE h.resource_id = r.id
                        AND h.anchor_table = 'kb_contexts'
                        AND h.anchor_id = $3::uuid))"#
    ))
    .bind(model)
    .bind(resource_filter)
    .bind(context_filter)
    .fetch_one(pool)
    .await?;

    Ok((resources as u64, chunks as u64))
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

/// What one claimed embed job did, so the caller can tally and decrement its budget without
/// re-reading the span. `chunks_embedded` is returned rather than the fn taking `&mut budget`, so
/// this stays a pure function of its inputs and the gate's assertions do not depend on mutation
/// order.
#[derive(Debug, Clone, Copy)]
enum EmbedJobResult {
    Deferred,
    /// `u64` to match `ChunkProgress::embedded` and `EmbedDispatchSummary::chunks_embedded`; the
    /// per-claim `budget` is `i64` and takes the cast at the decrement, exactly as before.
    Completed {
        chunks_embedded: u64,
    },
    Partial {
        chunks_embedded: u64,
    },
    Failed,
}

/// One claimed embed job, as its own span. The twin of `region_service::run_region_job`; see its doc
/// for why the per-job facts may not go on the tick span.
#[tracing::instrument(
    name = "embed_job",
    skip_all,
    fields(
        resource_id = %job.resource_id,
        attempts = job.attempts,
        queue_wait_ms = tracing::field::Empty,
        outcome = tracing::field::Empty,
        chunks_embedded = tracing::field::Empty,
    )
)]
async fn run_embed_job(
    pool: &PgPool,
    job: &ClaimedEmbedJob,
    queue_wait_ms: Option<i64>,
    past_deadline: bool,
    budget: i64,
) -> ApiResult<EmbedJobResult> {
    let span = tracing::Span::current();
    if let Some(wait) = queue_wait_ms {
        span.record("queue_wait_ms", wait);
    }

    let persona = Persona::Embed.as_str();
    let dispatch = DispatchType::Embed.as_str();

    if past_deadline {
        // Past the deadline: defer this job — re-enqueue untouched to resume next tick rather than
        // hold a lease (a held lease looks like a crash to the reaper and is not reclaimable for
        // DEFAULT_EMBED_LEASE_SECONDS). 0 chunks embedded, job resumed later.
        workflow_job_service::complete_resource(pool, job.resource_id, persona, dispatch).await?;
        workflow_job_service::enqueue_resource(pool, job.resource_id, persona, dispatch).await?;
        span.record("outcome", JobOutcome::Deferred.as_str());
        tracing::info!(
            "embed dispatch hit its wall-clock deadline; re-enqueued job for the next tick"
        );
        return Ok(EmbedJobResult::Deferred);
    }

    match temper_substrate::embed::embed_resource_chunks(pool, job.resource_id, budget).await {
        Ok(progress) => {
            span.record("chunks_embedded", progress.embedded);
            if progress.is_complete() {
                workflow_job_service::complete_resource(pool, job.resource_id, persona, dispatch)
                    .await?;
                span.record("outcome", JobOutcome::Completed.as_str());
                Ok(EmbedJobResult::Completed {
                    chunks_embedded: progress.embedded,
                })
            } else {
                // More stale chunks than this claim's budget: complete + re-enqueue so a later
                // iteration (this invocation, or the next tick) resumes it with a fresh budget.
                // Complete-then-enqueue (not hold the lease) keeps the reaper's attempt count clean
                // for large resources. The two writes are NOT atomic: a crash between them strands
                // the resource with no job — and there is no automatic stale sweep (`enqueue_stale`
                // is operator-triggered), so recovery is the next operator `temper admin reembed`
                // over a scope containing it.
                workflow_job_service::complete_resource(pool, job.resource_id, persona, dispatch)
                    .await?;
                workflow_job_service::enqueue_resource(pool, job.resource_id, persona, dispatch)
                    .await?;
                span.record("outcome", JobOutcome::Partial.as_str());
                tracing::info!(
                    embedded = progress.embedded,
                    remaining = progress.remaining,
                    "embed job partially drained; re-enqueued for the next tick"
                );
                Ok(EmbedJobResult::Partial {
                    chunks_embedded: progress.embedded,
                })
            }
        }
        Err(e) => {
            // Leave the job in_progress; the reaper's lease-expiry sweep retries it (then dead at
            // max attempts). One bad resource never aborts the pass.
            span.record("outcome", JobOutcome::Failed.as_str());
            let detail = e.to_string();
            tracing::warn!(error = %crate::error::bounded(&detail), "embed job failed; leaving for reaper retry");
            Ok(EmbedJobResult::Failed)
        }
    }
}

/// [`dispatch_tick`] with the wall-clock ceiling injected, so tests can force the deadline-defer path
/// deterministically (a `Duration::ZERO` defers every claimed job on its first check) without sleeping.
#[tracing::instrument(
    name = "embed_dispatch",
    skip_all,
    fields(
        backlog_depth = tracing::field::Empty,
        oldest_pending_age_ms = tracing::field::Empty,
        claimed = tracing::field::Empty,
        completed = tracing::field::Empty,
        deferred = tracing::field::Empty,
        failed = tracing::field::Empty,
        redriven = tracing::field::Empty,
        partial = tracing::field::Empty,
        chunks_embedded = tracing::field::Empty,
    )
)]
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

    // Before the first claim, deliberately — see `drain_span::read_queue_depth`.
    let span = tracing::Span::current();
    let depth = drain_span::read_queue_depth(pool, persona, dispatch).await?;
    span.record("backlog_depth", depth.backlog_depth);
    if let Some(age) = depth.oldest_pending_age_ms {
        span.record("oldest_pending_age_ms", age);
    }

    // `EmbedDispatchSummary.partial` counts BOTH the deadline-deferral and the budget-exhausted
    // resume — two different states (no work attempted vs. work done and resumed). The summary is a
    // wire type with consumers and is left alone; the span's `outcome` distinguishes them, and this
    // local is the deadline half so the tick span can report it separately.
    let mut deferred: u32 = 0;

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

        // One read for the whole batch, keyed by job id — see `drain_span::read_queue_waits`.
        let ids: Vec<Uuid> = claimed.iter().map(|j| j.id).collect();
        let waits = drain_span::read_queue_waits(pool, &ids).await?;

        for job in claimed {
            // Once past the deadline this stays true, so every remaining job in the batch defers —
            // the behaviour the old `continue` gave.
            let past_deadline = start.elapsed() >= deadline;
            match run_embed_job(
                pool,
                &job,
                waits.get(&job.id).copied(),
                past_deadline,
                budget,
            )
            .await?
            {
                EmbedJobResult::Deferred => {
                    deferred += 1;
                    summary.partial += 1;
                }
                EmbedJobResult::Completed { chunks_embedded } => {
                    summary.chunks_embedded += chunks_embedded;
                    budget -= chunks_embedded as i64;
                    summary.completed += 1;
                }
                EmbedJobResult::Partial { chunks_embedded } => {
                    summary.chunks_embedded += chunks_embedded;
                    budget -= chunks_embedded as i64;
                    summary.partial += 1;
                }
                EmbedJobResult::Failed => summary.failed += 1,
            }
        }

        // Stop looping once past the deadline — checked after a full claim, so ≥1 claim always runs.
        if start.elapsed() >= deadline {
            break;
        }
    }

    span.record("claimed", summary.claimed);
    span.record("completed", summary.completed);
    // The deadline half of `summary.partial`, separated — see the local's declaration.
    span.record("deferred", deferred);
    span.record("failed", summary.failed);
    span.record("redriven", summary.redriven);
    span.record("partial", summary.partial);
    span.record("chunks_embedded", summary.chunks_embedded);
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
mod tests;
