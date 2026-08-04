//! The region-clock drain: runs T6's two clocks off the request path (goal 019fc46c).
//!
//! A resource write no longer ticks the clocks itself — it enqueues an anchor-scoped job
//! (`DbBackend::queue_region_clocks`) and returns. This drain claims those jobs and runs
//! [`region_clocks::tick`] against each anchor, unchanged.
//!
//! ## Why this is the embed drain's twin and not something new
//!
//! It deliberately mirrors [`crate::services::embed_service::dispatch_tick`]: reap stale leases,
//! claim a capped batch, work each job under a wall-clock ceiling, complete or leave in-flight for
//! the reaper. That symmetry is the point — the two workers have the same failure modes (a slow
//! unit of work inside a serverless function with a hard timeout) and should not drift in how they
//! handle them.
//!
//! One shape is deliberately NOT copied. The embed drain re-enqueues a partially-drained resource to
//! resume with a fresh budget. A region tick has no partial state: it either settles the anchor to
//! the current watermark or it does not, and a re-tick after a completed settling is a cheap no-op
//! (the watermark advanced, so the formation count is back below threshold). So there is no
//! `partial` tally here — a job is completed, deferred whole, or left to the reaper.
//!
//! ## The deadline is not optional here
//!
//! A single materialization on a large anchor has been observed at 55–94 seconds in production. The
//! drain must therefore run on the `api/internal` function (`maxDuration: 300`), and it must stop
//! claiming well before that ceiling or the function is killed mid-transaction — which would abandon
//! the anchor row lock to Postgres to clean up on connection teardown, and leave the job's lease to
//! expire before anything retries it.

use serde::Serialize;
use sqlx::PgPool;

use crate::backend::region_clocks;
use crate::error::ApiResult;
use crate::services::drain_span::{self, JobOutcome};
use crate::services::workflow_job_service;
use temper_core::types::workflow_job::{ClaimedAnchorJob, DispatchType, Persona, RegionJobPayload};

/// Wall-clock ceiling for one drain invocation, in seconds. Sits well under the `api/internal`
/// function's `maxDuration: 300` so a claimed job that runs long still leaves room to complete and
/// return a summary rather than being killed mid-transaction.
const DEFAULT_REGION_DISPATCH_DEADLINE_SECONDS: u64 = 240;

/// Env override for the deadline above.
const REGION_DISPATCH_DEADLINE_ENV: &str = "TEMPER_REGION_DISPATCH_DEADLINE_SECONDS";

/// How many anchors one invocation claims per iteration. Deliberately small: anchors are few (four
/// carry production load) and each settling is expensive, so a large batch would only mean holding
/// leases on anchors this invocation has no time to reach.
const DEFAULT_REGION_DISPATCH_CAP: i32 = 2;

/// Lease for a claimed region job. MUST exceed the drain deadline so a job that is genuinely still
/// running never looks dead to the reaper and gets double-claimed.
const DEFAULT_REGION_LEASE_SECONDS: i32 = 600;

/// One-line summary of a region-drain pass.
#[derive(Debug, Default, Serialize)]
pub struct RegionDispatchSummary {
    /// Jobs claimed this invocation.
    pub claimed: u32,
    /// Anchors whose clocks ran and were marked done.
    pub completed: u32,
    /// Jobs deferred untouched because the invocation ran out of wall clock.
    pub deferred: u32,
    /// Jobs whose tick errored; left in-flight for the reaper to retry.
    pub failed: u32,
    /// Anchors whose formation clock actually materialized this pass (a subset of `completed` — the
    /// rest ticked below threshold, which is the cheap no-op case).
    pub materialized: u32,
    /// Anchors whose salience clock fired this pass.
    pub salience_refreshed: u32,
}

/// What one claimed job did, so the caller can tally without re-reading the span.
#[derive(Debug, Clone, Copy)]
enum RegionJobResult {
    Deferred,
    Completed {
        materialized: bool,
        salience_refreshed: bool,
    },
    Failed,
}

/// One claimed job, as its own span.
///
/// Extracted from the claim loop rather than inlined so the per-job facts land on a span of their
/// own. With no child span, `Span::current().record(..)` resolves to the *tick* span and appears to
/// work — right up until it silently doesn't. That is the trap CLAUDE.md's span-field convention
/// exists for, and `crates/temper-services/tests/drain_span_test.rs` asserts against here.
///
/// `past_deadline` is passed rather than recomputed inside, so the caller keeps ownership of the
/// invocation clock and this stays a pure function of its inputs.
#[tracing::instrument(
    name = "region_job",
    skip_all,
    fields(
        anchor_id = %job.anchor.uuid(),
        anchor_kind = job.anchor.table(),
        attempts = job.attempts,
        queue_wait_ms = tracing::field::Empty,
        outcome = tracing::field::Empty,
        materialized = tracing::field::Empty,
        salience_refreshed = tracing::field::Empty,
    )
)]
async fn run_region_job(
    pool: &PgPool,
    job: &ClaimedAnchorJob,
    queue_wait_ms: Option<i64>,
    past_deadline: bool,
) -> ApiResult<RegionJobResult> {
    let span = tracing::Span::current();
    if let Some(wait) = queue_wait_ms {
        span.record("queue_wait_ms", wait);
    }

    let persona = Persona::Region.as_str();
    let dispatch = DispatchType::Materialize.as_str();

    if past_deadline {
        // Past the deadline: hand this job back untouched rather than start a settling we cannot
        // finish. Complete-then-re-enqueue instead of holding the lease, so the reaper's attempt
        // count stays clean — the same reasoning the embed drain's deferral path gives.
        workflow_job_service::complete_anchor(pool, job.anchor, persona, dispatch).await?;
        workflow_job_service::enqueue_anchor(
            pool,
            job.anchor,
            persona,
            dispatch,
            RegionJobPayload {
                emitter: job.emitter,
            },
        )
        .await?;
        span.record("outcome", JobOutcome::Deferred.as_str());
        tracing::info!(
            "region dispatch hit its wall-clock deadline; re-enqueued job for the next tick"
        );
        return Ok(RegionJobResult::Deferred);
    }

    match region_clocks::tick(pool, job.anchor, job.emitter.into(), None).await {
        Ok(tick) => {
            workflow_job_service::complete_anchor(pool, job.anchor, persona, dispatch).await?;
            span.record("outcome", JobOutcome::Completed.as_str());
            span.record("materialized", tick.materialized);
            span.record("salience_refreshed", tick.salience_refreshed);
            tracing::debug!("region clocks ticked");
            Ok(RegionJobResult::Completed {
                materialized: tick.materialized,
                salience_refreshed: tick.salience_refreshed,
            })
        }
        Err(e) => {
            // Leave the job in_progress; the reaper's lease-expiry sweep retries it (then dead at
            // max attempts). One bad anchor never aborts the pass — and unlike the old inline tick,
            // a failure here is a job the queue still knows about rather than a warning nothing
            // tracks.
            span.record("outcome", JobOutcome::Failed.as_str());
            tracing::warn!(
                error = %e,
                "region clock tick failed; left in-flight for the reaper to retry"
            );
            Ok(RegionJobResult::Failed)
        }
    }
}

/// Resolve the invocation deadline from env, falling back to the default.
fn resolve_dispatch_deadline() -> std::time::Duration {
    let secs = std::env::var(REGION_DISPATCH_DEADLINE_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_REGION_DISPATCH_DEADLINE_SECONDS);
    std::time::Duration::from_secs(secs)
}

/// Run one region-drain pass.
pub async fn dispatch_tick(pool: &PgPool, cap: Option<i32>) -> ApiResult<RegionDispatchSummary> {
    dispatch_tick_inner(pool, cap, resolve_dispatch_deadline()).await
}

/// [`dispatch_tick`] with the wall-clock ceiling injected, so tests can force the deferral path
/// deterministically (`Duration::ZERO` defers every claimed job) without sleeping.
#[tracing::instrument(
    name = "region_dispatch",
    skip_all,
    fields(
        backlog_depth = tracing::field::Empty,
        oldest_pending_age_ms = tracing::field::Empty,
        claimed = tracing::field::Empty,
        completed = tracing::field::Empty,
        deferred = tracing::field::Empty,
        failed = tracing::field::Empty,
        materialized = tracing::field::Empty,
        salience_refreshed = tracing::field::Empty,
    )
)]
async fn dispatch_tick_inner(
    pool: &PgPool,
    cap: Option<i32>,
    deadline: std::time::Duration,
) -> ApiResult<RegionDispatchSummary> {
    let persona = Persona::Region.as_str();
    let dispatch = DispatchType::Materialize.as_str();

    // Reap stale leases (a killed function mid-settle → retry/dead) before claiming, mirroring the
    // embed and steward ticks.
    workflow_job_service::reap(pool, "region lease expired").await?;

    let cap = cap.unwrap_or(DEFAULT_REGION_DISPATCH_CAP);
    let mut summary = RegionDispatchSummary::default();
    let start = std::time::Instant::now();

    // Before the first claim, deliberately: this is "how deep was the queue when this tick arrived".
    // Read after the loop, it would describe the queue this tick has just drained — and a drain
    // falling behind would look healthy at exactly the moment it is not.
    let span = tracing::Span::current();
    let depth = drain_span::read_queue_depth(pool, persona, dispatch).await?;
    span.record("backlog_depth", depth.backlog_depth);
    if let Some(age) = depth.oldest_pending_age_ms {
        span.record("oldest_pending_age_ms", age);
    }

    // Do-while shape, as in the embed drain: always run at least one claim, then keep claiming until
    // the queue is empty or the deadline is hit. Checking the deadline AFTER a full claim means an
    // invocation can never spin, even under a pathologically small deadline, and preserves the
    // ZERO-deadline defer semantics the tests assert.
    loop {
        let claimed = workflow_job_service::claim_anchor(
            pool,
            persona,
            dispatch,
            cap,
            DEFAULT_REGION_LEASE_SECONDS,
        )
        .await?;
        if claimed.is_empty() {
            break; // queue drained — nothing left this invocation
        }
        summary.claimed += claimed.len() as u32;

        // One read for the whole batch, keyed by job id. The per-job wait is enqueue→lease, which
        // only the queue row knows; see `drain_span::read_queue_waits` for why this is a separate
        // SELECT rather than a widened claim function.
        let ids: Vec<uuid::Uuid> = claimed.iter().map(|j| j.id).collect();
        let waits = drain_span::read_queue_waits(pool, &ids).await?;

        for job in claimed {
            // Once past the deadline this stays true, so every remaining job in the batch defers —
            // the behaviour the old `continue` gave.
            let past_deadline = start.elapsed() >= deadline;
            match run_region_job(pool, &job, waits.get(&job.id).copied(), past_deadline).await? {
                RegionJobResult::Deferred => summary.deferred += 1,
                RegionJobResult::Completed {
                    materialized,
                    salience_refreshed,
                } => {
                    summary.completed += 1;
                    if materialized {
                        summary.materialized += 1;
                    }
                    if salience_refreshed {
                        summary.salience_refreshed += 1;
                    }
                }
                RegionJobResult::Failed => summary.failed += 1,
            }
        }

        if start.elapsed() >= deadline {
            break;
        }
    }

    span.record("claimed", summary.claimed);
    span.record("completed", summary.completed);
    span.record("deferred", summary.deferred);
    span.record("failed", summary.failed);
    span.record("materialized", summary.materialized);
    span.record("salience_refreshed", summary.salience_refreshed);
    Ok(summary)
}
