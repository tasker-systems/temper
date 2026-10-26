//! Shape reconciliation on the anchor-scoped job queue (spec §7.3, plan Task 6).
//!
//! Declaring a shape enqueues a reconcile job on the existing `kb_workflow_jobs` anchor queue —
//! no new queue infrastructure. The worker claims jobs and verdicts pre-existing artifacts by
//! running `jsonschema` validation against the shape in force.
//!
//! ## Authority gate
//!
//! The authority gate lives HERE, not in Beat E: spec §5 rules that declaring a shape requires
//! authority over its home (`context_authorable_by_profile` / `cogmap_authorable_by_profile`), and
//! Beats A–D otherwise ship a write path with no authorization check. The gate calls the
//! predicates, never restates them — the same two-arm branch `can_modify_resource` uses
//! (`migrations/20260804000020:102-108`).

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::drain_span::{self, JobOutcome};
use crate::services::shape_loader::{self, SchemaVerdict};
use crate::services::workflow_job_service;
use temper_core::types::artifact_ask::walk::MAX_SCHEMA_BYTES;
use temper_core::types::data_artifact::unstorable_nul;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ContextId, EntityId, ProfileId, ShapeId};
use temper_core::types::workflow_job::{AnchorJobPayload, ClaimedAnchorJob, DispatchType, Persona};
use temper_substrate::payloads::{AnchorRef, EnforcementMode, KindOwner};
use temper_substrate::shape_schema::{self, ShapeSchemaUncompilable};
use temper_substrate::writes::{self, DataArtifactRefusal, DeclareShapeParams};

/// Lease for a claimed shape-reconcile job. Same reasoning as the region lease: MUST exceed the
/// Vercel function timeout (300s) so a genuinely-running reconciliation never looks dead to the
/// reaper.
const DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS: i32 = 600;

/// How many anchor-keyed jobs one claim takes. One, because an anchor's pass can run to the
/// deadline: a larger claim holds leases on anchors this invocation will not reach, and hands them
/// back re-enqueued behind the anchor that used the time, so the same anchor is claimed first again
/// next pass. Claimed one at a time, a handed-back job goes behind every job already waiting, so
/// anchors take turns, and a pass killed mid-anchor strands no job but its own.
const DEFAULT_SHAPE_RECONCILE_CAP: i32 = 1;

/// The most anchor-keyed jobs a caller's `cap` may claim at once.
const MAX_SHAPE_RECONCILE_CAP: i32 = 10;

/// Parameters for [`declare_shape`] — the service-layer write that gates on authority, fires the
/// substrate declare, and enqueues a reconcile job.
#[derive(Debug)]
pub struct DeclareShapeServiceParams<'a> {
    pub home: AnchorRef,
    pub kind: &'a str,
    pub kind_owner: Option<KindOwner>,
    pub schema: &'a serde_json::Value,
    pub enforcement: EnforcementMode,
    pub principal: ProfileId,
    pub emitter: EntityId,
}

/// Declare a shape: authority gate → compile → substrate declare → enqueue reconcile job.
///
/// The authority gate is the two-arm branch from spec §5: `context_authorable_by_profile` for a
/// context-homed shape, `cogmap_authorable_by_profile` for a cogmap-homed shape. It calls the
/// predicates, never restates them. A principal who cannot author the home is refused with
/// `Forbidden` — the same denial `DbBackend::check_context_authorable` /
/// `DbBackend::check_cogmap_authorable` use.
///
/// The schema is then judged, before any write, by the decision the loader and the by-id read
/// share ([`shape_loader::judge`]), on the blocking pool under the same compile permits. A schema
/// larger than the walk reads ([`MAX_SCHEMA_BYTES`]) is refused before it is compiled, with a
/// sentence naming the bound; one within it that cannot be compiled (malformed, referring to a
/// document that is never fetched, or with a pattern past `shape_schema::MAX_PATTERN_BYTES`) is
/// refused with the no-fetch constructor's fixed sentence. Both are a `DataArtifactRefusal`, so
/// a schema no ask could use is refused when it is declared rather than stored. Before either, a
/// family name or a schema holding U+0000, which Postgres cannot store, is refused as one too,
/// naming the field ([`unstorable_nul`]), where binding it would have failed in the database.
///
/// After the declare succeeds, a reconcile job is enqueued on the anchor queue. The single-flight
/// index collapses N declarations into one job — the second enqueue returns `None`, not an error.
pub async fn declare_shape(pool: &PgPool, p: DeclareShapeServiceParams<'_>) -> ApiResult<ShapeId> {
    // Authority gate — the two-arm branch from spec §5. Call the predicates, never restate them.
    check_home_authorable(pool, p.principal, &p.home).await?;

    // Auth before writes, and the schema judged before the write: the gate above answers a
    // stranger `Forbidden` whatever the schema says, so the refusals below teach only an author.
    // A U+0000 no row can hold is refused first, by the field that holds it; then the schema is
    // measured before it is compiled — a schema past the walk's byte bound is never compiled.
    if let Some(refusal) = unstorable_nul(p.kind, "schema", p.schema) {
        return Err(ApiError::DataArtifactRefusal(refusal));
    }
    match shape_loader::judge(p.schema).await? {
        SchemaVerdict::Compiles => {}
        SchemaVerdict::Uncompilable => {
            return Err(ApiError::DataArtifactRefusal(
                ShapeSchemaUncompilable.to_string(),
            ))
        }
        SchemaVerdict::TooLarge => return Err(ApiError::DataArtifactRefusal(too_large())),
    }

    // Substrate declare (fires the event, projects the row). A typed refusal (the wrapper's
    // SQL vocabulary, or the declare gate itself) surfaces with its own words — never the
    // internal-error class, which is for server faults.
    let shape_id = writes::declare_shape(
        pool,
        DeclareShapeParams {
            home: p.home,
            kind: p.kind,
            kind_owner: p.kind_owner,
            schema: p.schema,
            enforcement: p.enforcement,
            emitter: p.emitter,
        },
    )
    .await
    .map_err(|e| match e.downcast_ref::<DataArtifactRefusal>() {
        Some(r) => ApiError::DataArtifactRefusal(r.message.clone()),
        None => ApiError::Internal(e.to_string()),
    })?;

    // Enqueue a reconcile job on the anchor queue. The single-flight index collapses N
    // declarations into one job; the second enqueue returns None, not an error.
    let anchor = anchor_from_ref(&p.home);
    workflow_job_service::enqueue_anchor(
        pool,
        anchor,
        Persona::Shape.as_str(),
        DispatchType::ShapeReconcile.as_str(),
        AnchorJobPayload {
            emitter: p.emitter.uuid(),
        },
    )
    .await?;

    Ok(shape_id)
}

/// The declare refusal for a schema past the walk's byte bound: names the bound, and carries no
/// compiler text — the schema was never compiled.
fn too_large() -> String {
    format!(
        "the shape's JSON Schema is larger than {MAX_SCHEMA_BYTES} bytes of compact JSON, the most \
         this server reads, so it is not compiled; declare a smaller schema"
    )
}

/// One shape-reconcile drain pass, as `api/internal`'s `/api/shape/reconcile` cron runs it.
#[derive(Debug, Default, Serialize)]
pub struct ShapeReconcileSummary {
    /// Jobs claimed this invocation.
    pub claimed: u32,
    /// Anchors whose whole backlog was verdicted, and whose job was marked done.
    pub completed: u32,
    /// Jobs handed back untouched because the invocation's deadline was already spent.
    pub deferred: u32,
    /// Anchors whose backlog was partly verdicted when the deadline came; re-enqueued to resume.
    pub partial: u32,
    /// Jobs left in flight for the reaper to retry: the pass errored, or an artifact's verdict
    /// could not be written.
    pub failed: u32,
    /// Anchors re-enqueued after completing because their backlog was not empty: a declare, an
    /// amendment or a rehome that arrived while the job ran, and coalesced into it.
    pub requeued: u32,
    /// Verdicts written this invocation: the work product, which a tick that claimed nothing must
    /// not be mistaken for.
    pub verdicts_written: u64,
    /// Artifacts whose verdict could not be written; each is named in a warning, and its anchor's
    /// job is left in flight.
    pub failed_artifacts: u32,
    /// Shapes in force found uncompilable this invocation, counted once per anchor pass. Their
    /// artifacts get no verdict and stay `declared_not_yet_checked`; each is named in a warning.
    pub uncompilable_shapes: u32,
    /// Backlog batches read this invocation.
    pub batches_read: u32,
}

/// What one backlog batch may hold, so an anchor's backlog is never read into memory whole.
#[derive(Debug, Clone, Copy)]
struct BatchBound {
    /// At most this many artifacts.
    rows: i64,
    /// At most this many bytes of content, by `content_bytes`, with one exception: the first
    /// artifact of a batch is always read, however large, so a batch always moves forward.
    bytes: i64,
}

/// The batch the production drain reads: at most 200 artifacts and 32 MB of content, or one
/// artifact larger than that alone. An artifact commit is bounded by the 25 MiB gated body limit.
const SHAPE_RECONCILE_BATCH: BatchBound = BatchBound {
    rows: 200,
    bytes: 32_000_000,
};

/// At most this many errors are recorded in one verdict's `detail`
/// ([`shape_schema::judge_masked`]).
const MAX_RECORDED_ERRORS: usize = 64;

/// How many backlog artifacts the probe after a pass counts before it stops. Past it, the job
/// span's `backlog_remaining` is this number and `backlog_remaining_capped` is true: at least this
/// many remain, not exactly this many.
const BACKLOG_PROBE_ROWS: i64 = 1_000;

/// Wall-clock ceiling for one drain invocation, in seconds: the region drain's, for its reason. It
/// sits under the `api/internal` function's `maxDuration: 300`, so a pass that runs long still has
/// room to hand its job back and return a summary rather than be killed mid-batch. The lease
/// (`DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS`) exceeds it, so a running pass never looks dead to the
/// reaper. The deadline is checked after every artifact, so a pass overruns it by at most one
/// artifact's check.
const DEFAULT_SHAPE_RECONCILE_DEADLINE_SECONDS: u64 = 240;

/// Env override for the deadline above.
const SHAPE_RECONCILE_DEADLINE_ENV: &str = "TEMPER_SHAPE_RECONCILE_DEADLINE_SECONDS";

fn resolve_dispatch_deadline() -> std::time::Duration {
    let secs = std::env::var(SHAPE_RECONCILE_DEADLINE_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_SHAPE_RECONCILE_DEADLINE_SECONDS);
    std::time::Duration::from_secs(secs)
}

/// The claim size a pass uses: the caller's, held to `1..=MAX_SHAPE_RECONCILE_CAP`, or the
/// default. The door refuses a `cap` below 1 before it reaches here.
fn resolve_cap(cap: Option<i32>) -> i32 {
    cap.unwrap_or(DEFAULT_SHAPE_RECONCILE_CAP)
        .clamp(1, MAX_SHAPE_RECONCILE_CAP)
}

/// Run one shape-reconcile drain pass: reap expired leases, then claim anchor-keyed jobs and
/// verdict each anchor's backlog against the shape in force, in bounded batches, until the queue
/// is empty or the deadline is spent.
///
/// A pass that claims nothing writes nothing: assert on `verdicts_written`, not on `Ok`.
pub async fn dispatch_tick(pool: &PgPool, cap: Option<i32>) -> ApiResult<ShapeReconcileSummary> {
    dispatch_tick_inner(
        pool,
        cap,
        resolve_dispatch_deadline(),
        SHAPE_RECONCILE_BATCH,
    )
    .await
}

/// [`dispatch_tick`] with the deadline and the batch injected, so tests can force the deferral and
/// resumption paths deterministically (`Duration::ZERO` defers every claimed job) without seeding a
/// production-sized backlog.
#[tracing::instrument(
    name = "shape_reconcile_dispatch",
    skip_all,
    fields(
        backlog_depth = tracing::field::Empty,
        oldest_pending_age_ms = tracing::field::Empty,
        claimed = tracing::field::Empty,
        completed = tracing::field::Empty,
        deferred = tracing::field::Empty,
        failed = tracing::field::Empty,
        partial = tracing::field::Empty,
        requeued = tracing::field::Empty,
        verdicts_written = tracing::field::Empty,
        failed_artifacts = tracing::field::Empty,
        uncompilable_shapes = tracing::field::Empty,
        batches_read = tracing::field::Empty,
    )
)]
async fn dispatch_tick_inner(
    pool: &PgPool,
    cap: Option<i32>,
    deadline: std::time::Duration,
    batch: BatchBound,
) -> ApiResult<ShapeReconcileSummary> {
    let persona = Persona::Shape.as_str();
    let dispatch = DispatchType::ShapeReconcile.as_str();

    // Reap stale leases before claiming, mirroring the region and embed drains.
    workflow_job_service::reap(pool, "shape reconcile lease expired").await?;

    let cap = resolve_cap(cap);
    let mut summary = ShapeReconcileSummary::default();
    let start = std::time::Instant::now();

    // Before the first claim, for `drain_span::read_queue_depth`'s reason.
    let span = tracing::Span::current();
    let depth = drain_span::read_queue_depth(pool, persona, dispatch).await?;
    span.record("backlog_depth", depth.backlog_depth);
    if let Some(age) = depth.oldest_pending_age_ms {
        span.record("oldest_pending_age_ms", age);
    }

    // The region drain's do-while: always one claim, then keep claiming until the queue is empty or
    // the deadline is spent. A job is handed back (deferred or partial) only once the deadline is
    // spent, and the loop ends there, so a handed-back job is never re-claimed by the pass that
    // handed it back. A completed anchor queued again (`settle`) is the other way back into the
    // queue; one this pass already ran is handed back untouched and ends the pass, so no anchor is
    // run twice in one pass and a backlog the pass cannot clear can never spin it to the deadline.
    let mut ran_this_pass: HashSet<Uuid> = HashSet::new();
    let mut ran_again = false;
    loop {
        let claimed = workflow_job_service::claim_anchor(
            pool,
            persona,
            dispatch,
            cap,
            DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS,
        )
        .await?;
        if claimed.is_empty() {
            break;
        }
        summary.claimed += claimed.len() as u32;

        let ids: Vec<Uuid> = claimed.iter().map(|j| j.id).collect();
        let waits = drain_span::read_queue_waits(pool, &ids).await?;

        for job in claimed {
            if ran_again || !ran_this_pass.insert(job.anchor.uuid()) {
                hand_back(pool, &job).await?;
                summary.deferred += 1;
                ran_again = true;
                continue;
            }
            let past_deadline = start.elapsed() >= deadline;
            let ran = run_reconcile_job(
                pool,
                &job,
                waits.get(&job.id).copied(),
                past_deadline,
                Pace {
                    start,
                    deadline,
                    batch,
                },
            )
            .await?;
            match ran {
                ReconcileJobResult::Deferred => summary.deferred += 1,
                ReconcileJobResult::Failed => summary.failed += 1,
                ReconcileJobResult::Ran { pass, settled } => {
                    match settled {
                        Settled::Completed { requeued } => {
                            summary.completed += 1;
                            if requeued {
                                summary.requeued += 1;
                            }
                        }
                        Settled::HandedBack => summary.partial += 1,
                        Settled::LeftInFlight => summary.failed += 1,
                    }
                    summary.verdicts_written += pass.verdicts_written;
                    summary.failed_artifacts += pass.failed_artifacts;
                    summary.uncompilable_shapes += pass.uncompilable.len() as u32;
                    summary.batches_read += pass.batches_read;
                }
            }
        }

        if ran_again || start.elapsed() >= deadline {
            break;
        }
    }

    span.record("claimed", summary.claimed);
    span.record("completed", summary.completed);
    span.record("deferred", summary.deferred);
    span.record("failed", summary.failed);
    span.record("partial", summary.partial);
    span.record("requeued", summary.requeued);
    span.record("verdicts_written", summary.verdicts_written);
    span.record("failed_artifacts", summary.failed_artifacts);
    span.record("uncompilable_shapes", summary.uncompilable_shapes);
    span.record("batches_read", summary.batches_read);
    Ok(summary)
}

/// The pass's clock and batch, carried to each anchor.
#[derive(Debug, Clone, Copy)]
struct Pace {
    start: std::time::Instant,
    deadline: std::time::Duration,
    batch: BatchBound,
}

impl Pace {
    fn spent(&self) -> bool {
        self.start.elapsed() >= self.deadline
    }
}

/// What one claimed job did.
#[derive(Debug)]
enum ReconcileJobResult {
    Deferred,
    Ran { pass: AnchorPass, settled: Settled },
    Failed,
}

/// What became of a job whose anchor ran a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Settled {
    /// Marked done; `requeued` when the backlog was not empty after it, so a successor was queued.
    Completed { requeued: bool },
    /// The deadline came first: done, and a successor queued to resume.
    HandedBack,
    /// An artifact's verdict could not be written: left in flight for the reaper to retry.
    LeftInFlight,
}

/// What one anchor's pass did.
#[derive(Debug, Default)]
struct AnchorPass {
    verdicts_written: u64,
    failed_artifacts: u32,
    /// The shapes in force found uncompilable, excluded from the rest of the pass and from the
    /// probe after it.
    uncompilable: Vec<Uuid>,
    batches_read: u32,
    /// Whether the backlog was exhausted. False when the deadline came first: the verdicts
    /// already written stand, and the re-enqueued job resumes from what is left.
    finished: bool,
}

#[tracing::instrument(
    name = "shape_reconcile_job",
    skip_all,
    fields(
        anchor_id = %job.anchor.uuid(),
        anchor_kind = job.anchor.table(),
        attempts = job.attempts,
        queue_wait_ms = tracing::field::Empty,
        outcome = tracing::field::Empty,
        verdicts_written = tracing::field::Empty,
        failed_artifacts = tracing::field::Empty,
        backlog_remaining = tracing::field::Empty,
        backlog_remaining_capped = tracing::field::Empty,
    )
)]
async fn run_reconcile_job(
    pool: &PgPool,
    job: &ClaimedAnchorJob,
    queue_wait_ms: Option<i64>,
    past_deadline: bool,
    pace: Pace,
) -> ApiResult<ReconcileJobResult> {
    let span = tracing::Span::current();
    if let Some(wait) = queue_wait_ms {
        span.record("queue_wait_ms", wait);
    }

    if past_deadline {
        // Hand the job back untouched. Complete-then-re-enqueue rather than holding the lease, so
        // the reaper's attempt count stays clean — the region drain's reasoning.
        hand_back(pool, job).await?;
        span.record("outcome", JobOutcome::Deferred.as_str());
        return Ok(ReconcileJobResult::Deferred);
    }

    match reconcile_one_anchor(pool, job.anchor, pace).await {
        Ok(pass) => {
            let settled = settle(pool, job, &pass).await?;
            span.record(
                "outcome",
                match settled {
                    Settled::Completed { .. } => JobOutcome::Completed.as_str(),
                    Settled::HandedBack => JobOutcome::Partial.as_str(),
                    Settled::LeftInFlight => JobOutcome::Failed.as_str(),
                },
            );
            span.record("verdicts_written", pass.verdicts_written);
            span.record("failed_artifacts", pass.failed_artifacts);
            Ok(ReconcileJobResult::Ran { pass, settled })
        }
        Err(e) => {
            // Leave the job in_progress; the reaper retries it. One bad anchor never aborts the
            // pass — the region drain's posture.
            span.record("outcome", JobOutcome::Failed.as_str());
            tracing::warn!(
                error = %e,
                "shape reconciliation failed for one anchor; left in-flight for the reaper"
            );
            Ok(ReconcileJobResult::Failed)
        }
    }
}

/// Settle a job after its anchor ran a pass, and record on the job span how much backlog remains.
///
/// - An artifact whose verdict could not be written leaves the job in flight, so the reaper
///   retries it: the pass has already verdicted everything else it reached, and the retry finds
///   only what is left.
/// - A pass the deadline cut off hands the job back to resume.
/// - A finished pass marks the job done, and then probes the backlog. The single-flight index
///   covers `in_progress`, so a declare, an amendment or a rehome that arrived while the job ran
///   coalesced into it, and its artifacts may sit behind the pass's cursor. Probed after the job
///   is done, a backlog that is not empty is queued again, and a later enqueue cannot coalesce
///   into the job just finished — the region drain's `requeue_if_erased_members` ordering.
///   Uncompilable shapes are excluded from the probe, or they would re-queue every pass.
async fn settle(pool: &PgPool, job: &ClaimedAnchorJob, pass: &AnchorPass) -> ApiResult<Settled> {
    let persona = Persona::Shape.as_str();
    let dispatch = DispatchType::ShapeReconcile.as_str();

    let settled = if pass.failed_artifacts > 0 {
        Settled::LeftInFlight
    } else if !pass.finished {
        hand_back(pool, job).await?;
        Settled::HandedBack
    } else {
        workflow_job_service::complete_anchor(pool, job.anchor, persona, dispatch).await?;
        Settled::Completed { requeued: false }
    };

    let probe = BatchBound {
        rows: BACKLOG_PROBE_ROWS + 1,
        bytes: i64::MAX,
    };
    let remaining = backlog_batch(pool, job.anchor, Uuid::nil(), &pass.uncompilable, probe)
        .await?
        .len() as i64;
    let span = tracing::Span::current();
    span.record("backlog_remaining", remaining.min(BACKLOG_PROBE_ROWS));
    span.record("backlog_remaining_capped", remaining > BACKLOG_PROBE_ROWS);

    if settled == (Settled::Completed { requeued: false }) && remaining > 0 {
        workflow_job_service::enqueue_anchor(
            pool,
            job.anchor,
            persona,
            dispatch,
            AnchorJobPayload {
                emitter: job.emitter,
            },
        )
        .await?;
        return Ok(Settled::Completed { requeued: true });
    }
    Ok(settled)
}

/// Mark the job done and enqueue its successor, so the next pass resumes the anchor.
///
/// **The two writes are not atomic.** A function killed between them leaves the anchor with no
/// job until the next declare or rehome reaches it — the region drain's deferral carries the same
/// gap.
async fn hand_back(pool: &PgPool, job: &ClaimedAnchorJob) -> ApiResult<()> {
    let persona = Persona::Shape.as_str();
    let dispatch = DispatchType::ShapeReconcile.as_str();
    workflow_job_service::complete_anchor(pool, job.anchor, persona, dispatch).await?;
    workflow_job_service::enqueue_anchor(
        pool,
        job.anchor,
        persona,
        dispatch,
        AnchorJobPayload {
            emitter: job.emitter,
        },
    )
    .await?;
    Ok(())
}

/// One backlog artifact, on cheap columns only: which artifact, under which shape.
#[derive(Debug)]
struct Candidate {
    artifact_id: Uuid,
    content_hash: String,
    shape_id: Uuid,
    shape_version: i32,
}

/// The anchor's backlog after `after`, in artifact-id order, held to `batch`.
///
/// The backlog is the live artifacts whose shape in force is homed at `anchor` and whose staleness
/// triple `(shape_id, shape_version, content_hash)` matches no stored verdict, less the shapes in
/// `skip`. An erased resource's artifacts are not backlog: the act empties their content but
/// leaves them unfolded with their hash (D2 step 5), so a shape declared or amended after the act
/// would select them, and the verdict writer refuses them (`_resource_write_guard`, D13).
///
/// Uses _data_artifact_shape_in_force (which calls _data_artifact_anchor and
/// _data_artifact_kind_owner) rather than re-deriving the home — the cogmap tiebreak is
/// load-bearing (20260820000020:186-188). The home-row EXISTS only narrows the scan to this
/// anchor's resources: the shape in force is homed at the home `_data_artifact_anchor` picks,
/// which is one of the resource's own, so no artifact the shape join admits is lost.
///
/// The row cap is applied first, so the window sums at most `batch.rows` rows. The byte cap is on
/// the running total including each row, and admits the first row whatever its size, so a batch
/// is a prefix of the backlog that always moves forward.
async fn backlog_batch(
    pool: &PgPool,
    anchor: HomeAnchor,
    after: Uuid,
    skip: &[Uuid],
    batch: BatchBound,
) -> ApiResult<Vec<Candidate>> {
    let rows = sqlx::query_as!(
        Candidate,
        r#"
        SELECT w.artifact_id     AS "artifact_id!: Uuid",
               w.content_hash    AS "content_hash!: String",
               w.shape_id        AS "shape_id!: Uuid",
               w.shape_version   AS "shape_version!: i32"
          FROM (SELECT b.*,
                       row_number() OVER read_order AS n,
                       sum(b.content_bytes) OVER read_order AS bytes_through
                  FROM (SELECT a.id           AS artifact_id,
                               a.content_hash,
                               a.content_bytes,
                               s.shape_id,
                               s.shape_version
                          FROM kb_data_artifacts a
                         CROSS JOIN LATERAL _data_artifact_shape_in_force(
                             a.resource_id, a.kind_owner_table, a.kind_owner_id, a.artifact_kind
                         ) s
                          JOIN kb_data_artifact_shapes sh ON sh.id = s.shape_id
                           AND sh.home_anchor_table = $1
                           AND sh.home_anchor_id    = $2
                          JOIN kb_resources r ON r.id = a.resource_id
                         WHERE NOT a.is_folded
                           AND r.erased_at IS NULL
                           AND a.id > $3
                           AND NOT (s.shape_id = ANY($4))
                           AND EXISTS (
                               SELECT 1 FROM kb_resource_homes h
                                WHERE h.resource_id  = a.resource_id
                                  AND h.anchor_table = $1
                                  AND h.anchor_id    = $2
                           )
                           AND NOT EXISTS (
                               SELECT 1 FROM kb_data_artifact_verdicts v
                                WHERE v.artifact_id   = a.id
                                  AND v.shape_id      = s.shape_id
                                  AND v.shape_version = s.shape_version
                                  AND v.content_hash  = a.content_hash
                           )
                         ORDER BY a.id
                         LIMIT $5) AS b
                WINDOW read_order AS (ORDER BY b.artifact_id ROWS UNBOUNDED PRECEDING)) AS w
         WHERE w.n = 1 OR w.bytes_through <= $6::bigint
         ORDER BY w.artifact_id
        "#,
        anchor.table(),
        anchor.uuid(),
        after,
        skip,
        batch.rows,
        batch.bytes,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Verdict one anchor's backlog, a bounded batch at a time, until it is exhausted or the pass's
/// deadline is spent.
///
/// The batches walk the backlog in artifact-id order from the start, so each moves forward even
/// past artifacts left unverdicted. The deadline is checked after every artifact, so the first is
/// always reached and a pass overruns the deadline by at most one artifact's check.
///
/// - **A shape that cannot be compiled** gives no verdict: its artifacts stay
///   `declared_not_yet_checked`, it is named once in a warning and counted, and the rest of the pass
///   excludes it (task 01a15661-8460-739d-9ed3-f1e8ccbf9b69).
/// - **An artifact whose verdict cannot be written** is named in a warning and counted, and the pass
///   goes on past it, so one such artifact never holds back the rest of its anchor.
/// - **Compiles and checks run on the blocking pool** under the shape compile permits
///   ([`shape_loader::run_off_the_executor`]), not on the executor: a shape's author chooses what a
///   check costs, and the drain is shared.
/// - **A verdict records its errors masked and bounded** ([`shape_schema::judge_masked`]), so its
///   `detail` never copies the artifact's content and its size is not the schema's to choose.
async fn reconcile_one_anchor(
    pool: &PgPool,
    anchor: HomeAnchor,
    pace: Pace,
) -> ApiResult<AnchorPass> {
    let mut pass = AnchorPass::default();
    let mut after = Uuid::nil();
    let mut validators = HashMap::new();

    loop {
        let candidates = backlog_batch(pool, anchor, after, &pass.uncompilable, pace.batch).await?;
        let Some(last) = candidates.last() else {
            pass.finished = true;
            return Ok(pass);
        };
        after = last.artifact_id;
        pass.batches_read += 1;

        // The batch's content, read once. An artifact committed with JSON `null` content has no
        // content row (20260820000020, `data_artifact_commit`), and the commit validated that
        // `null` against the shape, so reconciliation does the same.
        let ids: Vec<Uuid> = candidates.iter().map(|c| c.artifact_id).collect();
        let mut contents: HashMap<Uuid, serde_json::Value> = sqlx::query!(
            r#"
            SELECT artifact_id AS "artifact_id!: Uuid",
                   content     AS "content!: serde_json::Value"
              FROM kb_data_artifact_content
             WHERE artifact_id = ANY($1)
            "#,
            &ids[..],
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| (r.artifact_id, r.content))
        .collect();

        for c in candidates {
            if pass.uncompilable.contains(&c.shape_id) {
                continue;
            }
            let key = (c.shape_id, c.shape_version);
            if let Entry::Vacant(slot) = validators.entry(key) {
                let schema = sqlx::query_scalar!(
                    r#"SELECT schema AS "schema!: serde_json::Value"
                         FROM kb_data_artifact_shapes WHERE id = $1"#,
                    c.shape_id,
                )
                .fetch_one(pool)
                .await?;
                let compiled = shape_loader::run_off_the_executor(move || {
                    shape_schema::compile(&schema).map(Arc::new)
                })
                .await?;
                match compiled {
                    Ok(validator) => {
                        slot.insert(validator);
                    }
                    Err(ShapeSchemaUncompilable) => {
                        tracing::warn!(
                            shape_id = %c.shape_id,
                            shape_version = c.shape_version,
                            anchor = %anchor.uuid(),
                            "shape reconciliation skipped a shape that cannot be compiled; its \
                             artifacts stay declared_not_yet_checked"
                        );
                        pass.uncompilable.push(c.shape_id);
                        continue;
                    }
                }
            }
            let validator = Arc::clone(&validators[&key]);
            let content = contents
                .remove(&c.artifact_id)
                .unwrap_or(serde_json::Value::Null);
            let artifact_id = c.artifact_id;
            match verdict_one(pool, c, validator, content).await {
                Ok(()) => pass.verdicts_written += 1,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        artifact_id = %artifact_id,
                        anchor = %anchor.uuid(),
                        "shape reconciliation could not write one artifact's verdict; the pass \
                         goes on, and the anchor's job is left in flight for the reaper"
                    );
                    pass.failed_artifacts += 1;
                }
            }

            if pace.spent() {
                return Ok(pass);
            }
        }
    }
}

/// Check one artifact's content against its shape off the executor, and write the verdict.
async fn verdict_one(
    pool: &PgPool,
    c: Candidate,
    validator: Arc<shape_schema::Validator>,
    content: serde_json::Value,
) -> ApiResult<()> {
    let judged = shape_loader::run_off_the_executor(move || {
        shape_schema::judge_masked(&validator, &content, MAX_RECORDED_ERRORS)
    })
    .await?;
    sqlx::query!(
        "SELECT data_artifact_verdict_upsert($1,$2,$3,$4,$5,$6)",
        c.artifact_id,
        c.shape_id,
        c.shape_version,
        c.content_hash,
        judged.satisfied,
        judged.detail,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// The authority gate: call `context_authorable_by_profile` / `cogmap_authorable_by_profile` for
/// the home anchor, never restate them. Returns `Forbidden` on denial — the same denial
/// `DbBackend::check_context_authorable` / `DbBackend::check_cogmap_authorable` use.
async fn check_home_authorable(
    pool: &PgPool,
    principal: ProfileId,
    home: &AnchorRef,
) -> ApiResult<()> {
    let can = match home.table {
        temper_substrate::payloads::AnchorTable::Contexts => {
            sqlx::query_scalar!(
                "SELECT context_authorable_by_profile($1, $2)",
                *principal,
                home.id,
            )
            .fetch_one(pool)
            .await?
        }
        temper_substrate::payloads::AnchorTable::Cogmaps => {
            sqlx::query_scalar!(
                "SELECT cogmap_authorable_by_profile($1, $2)",
                *principal,
                home.id,
            )
            .fetch_one(pool)
            .await?
        }
        _ => {
            return Err(ApiError::BadRequest(
                "shape home must be a context or cogmap".to_string(),
            ))
        }
    };

    if can.unwrap_or(false) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Convert an `AnchorRef` to a `HomeAnchor`. The `AnchorRef` carries the raw `(table, id)` pair;
/// `HomeAnchor` is the typed closed two-variant enum.
fn anchor_from_ref(r: &AnchorRef) -> HomeAnchor {
    match r.table {
        temper_substrate::payloads::AnchorTable::Contexts => {
            HomeAnchor::Context(ContextId::from(r.id))
        }
        temper_substrate::payloads::AnchorTable::Cogmaps => {
            HomeAnchor::Cogmap(CogmapId::from(r.id))
        }
        other => panic!("shape home anchor must be context or cogmap, got {other:?}"),
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use temper_substrate::events::{fire, EventContext, Fired, SeedAction};
    use temper_substrate::ids::{DataArtifactId, ResourceId};
    use temper_substrate::payloads::{ArtifactIntent, ShapeState};
    use temper_substrate::scenario::bootseed;
    use temper_substrate::writes::{self, CreateParams};

    // ── seeding helpers ──────────────────────────────────────────────────────

    async fn insert_profile(pool: &PgPool, handle: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn insert_entity(pool: &PgPool, profile: Uuid, name: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id",
        )
        .bind(profile)
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn insert_context(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
        )
        .bind(owner)
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn make_resource(
        pool: &PgPool,
        owner: ProfileId,
        emitter: EntityId,
        home: AnchorRef,
        title: &str,
    ) -> ResourceId {
        writes::create_resource_with(
            pool,
            CreateParams {
                idempotency_key: None,
                title,
                origin_uri: title,
                body: "seed body",
                doc_type: "research",
                home,
                owner,
                originator: owner,
                emitter,
                properties: &[],
                chunks: None,
                sources: vec![],
            },
            EventContext::default(),
        )
        .await
        .unwrap()
    }

    /// Seed a world with one context-homed resource owned by `owner`.
    struct World {
        owner: Uuid,
        emitter: Uuid,
        context: Uuid,
        resource: Uuid,
    }

    async fn seed_world(pool: &PgPool, slug: &str) -> World {
        bootseed::seed_system(pool).await.unwrap();
        let owner = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
            .fetch_one(pool)
            .await
            .unwrap();
        let emitter = insert_entity(pool, owner, "test-emitter").await;
        let context = insert_context(pool, owner, slug).await;
        let resource = make_resource(
            pool,
            ProfileId::from(owner),
            EntityId::from(emitter),
            AnchorRef::context(ContextId::from(context)),
            "subject",
        )
        .await;
        World {
            owner,
            emitter,
            context,
            resource: resource.uuid(),
        }
    }

    /// Commit one data artifact via `fire()` and return the artifact id + conformance verdict.
    async fn commit_artifact(
        pool: &PgPool,
        resource: ResourceId,
        kind: &str,
        content: &serde_json::Value,
        emitter: Uuid,
    ) -> (DataArtifactId, ShapeState) {
        let mut tx = pool.begin().await.unwrap();
        let fired = fire(
            &mut tx,
            SeedAction::DataArtifactCommit {
                resource,
                kind,
                kind_owner: None,
                intent: ArtifactIntent::Current,
                precedence: 0.0,
                content,
                supersedes: &[],
                emitter: EntityId::from(emitter),
            },
        )
        .await
        .unwrap();
        let (id, shape_state) = match fired {
            Fired::DataArtifact {
                artifact,
                shape_state,
            } => (artifact, shape_state),
            other => panic!("expected Fired::DataArtifact, got {other:?}"),
        };
        tx.commit().await.unwrap();
        (id, shape_state)
    }

    fn string_value_schema() -> serde_json::Value {
        serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            },
            "required": ["value"]
        })
    }

    /// Count shape-reconcile jobs on the anchor queue for a given anchor.
    async fn reconcile_job_count(pool: &PgPool, anchor_table: &str, anchor_id: Uuid) -> i64 {
        let (cogmap, context) = match anchor_table {
            "kb_cogmaps" => (Some(anchor_id), None::<Uuid>),
            _ => (None, Some(anchor_id)),
        };
        sqlx::query_scalar(
            "SELECT count(*) FROM kb_workflow_jobs \
             WHERE cogmap_id IS NOT DISTINCT FROM $1 \
               AND context_id IS NOT DISTINCT FROM $2 \
               AND persona = 'shape' \
               AND dispatch_type = 'shape-reconcile'",
        )
        .bind(cogmap)
        .bind(context)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Read the shape_state from the SQL read function for the first live artifact of `resource`.
    async fn read_shape_state(
        pool: &PgPool,
        principal: ProfileId,
        resource: ResourceId,
    ) -> Option<ShapeState> {
        let retrieved = temper_substrate::readback::artifacts_for_resource(
            pool, principal, resource, None, None, false,
        )
        .await
        .unwrap();
        retrieved.first().map(|a| a.shape_state)
    }

    // ── witnesses ─────────────────────────────────────────────────────────────

    /// Declaring a shape enqueues exactly one reconcile job, anchored to the shape's home.
    #[sqlx::test(migrations = "../../migrations")]
    async fn declaring_a_shape_enqueues_one_reconcile_job(pool: PgPool) {
        let w = seed_world(&pool, "enqueue-test").await;
        let kind = "measurement";
        let schema = string_value_schema();

        let shape_id = declare_shape(
            &pool,
            DeclareShapeServiceParams {
                home: AnchorRef::context(ContextId::from(w.context)),
                kind,
                kind_owner: None,
                schema: &schema,
                enforcement: EnforcementMode::Advisory,
                principal: ProfileId::from(w.owner),
                emitter: EntityId::from(w.emitter),
            },
        )
        .await
        .unwrap();

        assert!(!shape_id.uuid().is_nil(), "shape was declared");
        assert_eq!(
            reconcile_job_count(&pool, "kb_contexts", w.context).await,
            1,
            "exactly one shape-reconcile job anchored to the shape's home"
        );
    }

    /// Declaring twice collapses to one in-flight job — the single-flight index does its job.
    /// The second enqueue returns `None`, not an error.
    #[sqlx::test(migrations = "../../migrations")]
    async fn declaring_twice_collapses_to_one_in_flight_job(pool: PgPool) {
        let w = seed_world(&pool, "dedup-test").await;
        let kind = "measurement";
        let schema = string_value_schema();
        let home = AnchorRef::context(ContextId::from(w.context));

        // First declare — enqueues a job.
        declare_shape(
            &pool,
            DeclareShapeServiceParams {
                home,
                kind,
                kind_owner: None,
                schema: &schema,
                enforcement: EnforcementMode::Advisory,
                principal: ProfileId::from(w.owner),
                emitter: EntityId::from(w.emitter),
            },
        )
        .await
        .unwrap();

        // Second declare — the single-flight index collapses: still one job, not two.
        declare_shape(
            &pool,
            DeclareShapeServiceParams {
                home,
                kind,
                kind_owner: None,
                schema: &schema,
                enforcement: EnforcementMode::Advisory,
                principal: ProfileId::from(w.owner),
                emitter: EntityId::from(w.emitter),
            },
        )
        .await
        .unwrap();

        assert_eq!(
            reconcile_job_count(&pool, "kb_contexts", w.context).await,
            1,
            "two declarations collapse to one in-flight job — the single-flight index works"
        );
    }

    /// Reconciliation verdicts the pre-existing backlog: artifacts committed BEFORE the declaration
    /// move from `DeclaredNotYetChecked` to a real verdict.
    #[sqlx::test(migrations = "../../migrations")]
    async fn reconciliation_verdicts_the_pre_existing_backlog(pool: PgPool) {
        let w = seed_world(&pool, "reconcile-test").await;
        let kind = "measurement";
        let resource = ResourceId::from(w.resource);

        // Commit an artifact BEFORE declaring any shape — it starts as NeverDeclared.
        let conforming = serde_json::json!({"value": "42"});
        let (_artifact_id, shape_state) =
            commit_artifact(&pool, resource, kind, &conforming, w.emitter).await;
        assert_eq!(
            shape_state,
            ShapeState::NeverDeclared,
            "commit before any shape declared must be NeverDeclared"
        );

        // Declare a shape — this enqueues a reconcile job.
        let schema = string_value_schema();
        declare_shape(
            &pool,
            DeclareShapeServiceParams {
                home: AnchorRef::context(ContextId::from(w.context)),
                kind,
                kind_owner: None,
                schema: &schema,
                enforcement: EnforcementMode::Advisory,
                principal: ProfileId::from(w.owner),
                emitter: EntityId::from(w.emitter),
            },
        )
        .await
        .unwrap();

        // The pre-existing artifact now reads as DeclaredNotYetChecked — a shape is in force but
        // no verdict has been recorded yet.
        let read_state = read_shape_state(&pool, ProfileId::from(w.owner), resource)
            .await
            .expect("artifact should be visible");
        assert_eq!(
            read_state,
            ShapeState::DeclaredNotYetChecked,
            "after declare, the pre-existing artifact must read as DeclaredNotYetChecked"
        );

        // Run the reconciler — it should verdict the backlog.
        let count = dispatch_tick(&pool, None).await.unwrap().verdicts_written;

        assert_eq!(
            count, 1,
            "exactly one verdict must be written — the pre-existing artifact"
        );

        // The artifact now reads as DeclaredSatisfied — the conforming content was validated.
        let read_state = read_shape_state(&pool, ProfileId::from(w.owner), resource)
            .await
            .expect("artifact should still be visible");
        assert_eq!(
            read_state,
            ShapeState::DeclaredSatisfied,
            "after reconciliation, the conforming artifact must read as DeclaredSatisfied"
        );
    }

    /// A reconcile tick that claims zero jobs must not be mistakable for a successful sweep —
    /// assert on verdicts written, not on the tick returning Ok. An empty anchor (no jobs queued)
    /// must write zero verdicts.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_reconcile_tick_with_no_work_touches_nothing(pool: PgPool) {
        let w = seed_world(&pool, "no-work-test").await;
        let kind = "measurement";
        let resource = ResourceId::from(w.resource);

        // Commit an artifact but do NOT declare a shape — no reconcile job is queued.
        let conforming = serde_json::json!({"value": "42"});
        commit_artifact(&pool, resource, kind, &conforming, w.emitter).await;

        // No jobs on the queue.
        assert_eq!(
            reconcile_job_count(&pool, "kb_contexts", w.context).await,
            0,
            "no shape declared → no reconcile job queued"
        );

        // Run the reconciler — it claims zero jobs and writes zero verdicts.
        let count = dispatch_tick(&pool, None).await.unwrap().verdicts_written;

        assert_eq!(
            count, 0,
            "a tick with no work must write zero verdicts — not be mistakable for a successful sweep"
        );

        // No verdict rows exist.
        let verdict_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_verdicts")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            verdict_count, 0,
            "no verdict rows must exist when no reconciliation ran"
        );
    }

    /// A principal who cannot author the home context is refused — the authority gate is
    /// load-bearing. A stranger (no team membership, no ownership) gets `Forbidden`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn declaring_a_shape_requires_authority_over_the_home(pool: PgPool) {
        let w = seed_world(&pool, "auth-gate-test").await;
        let kind = "measurement";
        let schema = string_value_schema();

        // A stranger — no team membership, no ownership of the home context.
        let stranger = insert_profile(&pool, "shape-stranger").await;

        let result = declare_shape(
            &pool,
            DeclareShapeServiceParams {
                home: AnchorRef::context(ContextId::from(w.context)),
                kind,
                kind_owner: None,
                schema: &schema,
                enforcement: EnforcementMode::Advisory,
                principal: ProfileId::from(stranger),
                emitter: EntityId::from(w.emitter),
            },
        )
        .await;

        assert!(
            result.is_err(),
            "a stranger who cannot author the home context must be refused"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, ApiError::Forbidden),
            "the refusal must be Forbidden, got {err:?}"
        );

        // No shape was declared and no job was enqueued.
        let shape_count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_shapes")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(shape_count, 0, "no shape row must exist after a refusal");

        assert_eq!(
            reconcile_job_count(&pool, "kb_contexts", w.context).await,
            0,
            "no reconcile job must be enqueued after a refusal"
        );

        // The owner CAN declare — the gate admits them.
        declare_shape(
            &pool,
            DeclareShapeServiceParams {
                home: AnchorRef::context(ContextId::from(w.context)),
                kind,
                kind_owner: None,
                schema: &schema,
                enforcement: EnforcementMode::Advisory,
                principal: ProfileId::from(w.owner),
                emitter: EntityId::from(w.emitter),
            },
        )
        .await
        .expect("the owner must be able to declare a shape in their own context");
    }

    /// Run the erasure act on `resource` as `operator`, through the SQL act directly (the service
    /// gate is the admin surface's concern, not reconcile's).
    async fn erase(pool: &PgPool, resource: Uuid, operator: Uuid, emitter: Uuid) {
        sqlx::query("SELECT resource_erasure_execute($1,$2,$3,$4)")
            .bind(resource)
            .bind(operator)
            .bind(emitter)
            .bind(Uuid::now_v7())
            .execute(pool)
            .await
            .expect("the erasure act completes");
    }

    /// The `shape_version` of the verdict stored for `artifact`, if any.
    async fn verdict_version(pool: &PgPool, artifact: DataArtifactId) -> Option<i32> {
        sqlx::query_scalar(
            "SELECT shape_version FROM kb_data_artifact_verdicts WHERE artifact_id = $1",
        )
        .bind(artifact.uuid())
        .fetch_optional(pool)
        .await
        .unwrap()
    }

    /// The statuses of every shape-reconcile job on a context anchor.
    async fn reconcile_job_statuses(pool: &PgPool, context: Uuid) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT status::text FROM kb_workflow_jobs \
              WHERE context_id = $1 AND persona = 'shape' AND dispatch_type = 'shape-reconcile' \
              ORDER BY enqueued_at, id",
        )
        .bind(context)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// An erased resource's artifact never blocks reconcile for its home. The guarded verdict
    /// writer refuses it (D13), so the backlog must not reach it: the sibling is verdicted, the
    /// anchor's job completes, and the erased artifact gains no verdict.
    ///
    /// The erased resource's artifact is committed first, so it leads the backlog; a filter that
    /// merely let the sibling go first would still leave the anchor in flight, which the job
    /// status and the returned count catch whatever the order.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_erased_resources_artifact_never_blocks_reconcile_for_its_home(pool: PgPool) {
        let w = seed_world(&pool, "erased-reconcile").await;
        let kind = "measurement";
        let home = AnchorRef::context(ContextId::from(w.context));
        let erased = ResourceId::from(w.resource);
        let sibling = make_resource(
            &pool,
            ProfileId::from(w.owner),
            EntityId::from(w.emitter),
            home,
            "sibling",
        )
        .await;

        // 1. A shape in force in the home.
        let declare = |schema: serde_json::Value| {
            let pool = pool.clone();
            async move {
                declare_shape(
                    &pool,
                    DeclareShapeServiceParams {
                        home,
                        kind,
                        kind_owner: None,
                        schema: &schema,
                        enforcement: EnforcementMode::Advisory,
                        principal: ProfileId::from(w.owner),
                        emitter: EntityId::from(w.emitter),
                    },
                )
                .await
                .unwrap();
            }
        };
        declare(string_value_schema()).await;
        assert_eq!(
            dispatch_tick(&pool, None).await.unwrap().verdicts_written,
            0,
            "the first declare's job finds an empty backlog"
        );

        // 2. One artifact of the family on each resource, the erased-to-be one first.
        let content = serde_json::json!({"value": "42"});
        let (erased_artifact, _) = commit_artifact(&pool, erased, kind, &content, w.emitter).await;
        let (sibling_artifact, _) =
            commit_artifact(&pool, sibling, kind, &content, w.emitter).await;

        // 3. Erase the first resource.
        erase(&pool, w.resource, w.owner, w.emitter).await;
        let erased_before = verdict_version(&pool, erased_artifact).await;

        // 4. Amend the shape: both triples go stale, so both artifacts are in the backlog.
        let mut amended = string_value_schema();
        amended["properties"]["unit"] = serde_json::json!({ "type": "string" });
        declare(amended).await;

        let written = dispatch_tick(&pool, None).await.unwrap().verdicts_written;

        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["done".to_owned(), "done".to_owned()],
            "the amended shape's reconcile job completes; it is not left in flight for the reaper"
        );
        assert_eq!(written, 1, "exactly one verdict is written: the sibling's");
        assert_eq!(
            verdict_version(&pool, sibling_artifact).await,
            Some(2),
            "the sibling is verdicted against the amended shape"
        );
        assert_eq!(
            verdict_version(&pool, erased_artifact).await,
            erased_before,
            "the erased resource's artifact gains no verdict"
        );
    }

    /// Declare the world's `measurement` shape over a backlog of `n` artifacts, one per resource,
    /// committed before the declare. Returns the artifacts in commit order.
    async fn backlog_of(pool: &PgPool, w: &World, n: usize) -> Vec<DataArtifactId> {
        let home = AnchorRef::context(ContextId::from(w.context));
        let content = serde_json::json!({"value": "42"});
        let mut artifacts = Vec::with_capacity(n);
        for i in 0..n {
            let resource = make_resource(
                pool,
                ProfileId::from(w.owner),
                EntityId::from(w.emitter),
                home,
                &format!("backlog-{i}"),
            )
            .await;
            let (artifact, _) =
                commit_artifact(pool, resource, "measurement", &content, w.emitter).await;
            artifacts.push(artifact);
        }
        declare_shape(
            pool,
            DeclareShapeServiceParams {
                home,
                kind: "measurement",
                kind_owner: None,
                schema: &string_value_schema(),
                enforcement: EnforcementMode::Advisory,
                principal: ProfileId::from(w.owner),
                emitter: EntityId::from(w.emitter),
            },
        )
        .await
        .unwrap();
        artifacts
    }

    const ROOMY: std::time::Duration = std::time::Duration::from_secs(600);

    /// FAILS IF: a pass reads one batch and stops, so a backlog larger than a batch is left
    /// partly unverdicted with its job marked done.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_backlog_larger_than_a_batch_is_verdicted_across_batches(pool: PgPool) {
        let w = seed_world(&pool, "batched-reconcile").await;
        backlog_of(&pool, &w, 5).await;

        let summary = dispatch_tick_inner(
            &pool,
            None,
            ROOMY,
            BatchBound {
                rows: 2,
                bytes: i64::MAX,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            summary.verdicts_written, 5,
            "every artifact of the backlog is verdicted"
        );
        assert_eq!(summary.completed, 1, "the anchor's job completes");
        assert_eq!(summary.partial, 0);
        assert_eq!(
            summary.batches_read, 3,
            "a two-row bound reads five artifacts as batches of two, two and one"
        );
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["done".to_owned()]
        );
    }

    /// FAILS IF: the byte bound admits no row when the first artifact alone is over it, so a batch
    /// of large artifacts never moves forward; or admits a row past the bound after the first, so
    /// the bound does not bound the batch.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_batch_always_admits_its_first_artifact_whatever_its_size(pool: PgPool) {
        let w = seed_world(&pool, "byte-bounded-reconcile").await;
        backlog_of(&pool, &w, 3).await;

        let summary = dispatch_tick_inner(
            &pool,
            None,
            ROOMY,
            BatchBound {
                rows: 100,
                bytes: 1,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            summary.verdicts_written, 3,
            "a one-byte budget still reaches every artifact"
        );
        assert_eq!(
            summary.batches_read, 3,
            "a one-byte budget reads exactly one artifact a batch"
        );
        assert_eq!(summary.completed, 1);
    }

    /// FAILS IF: the byte bound admits the artifact that crosses it, so a batch can hold up to one
    /// artifact more than the bound says.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_byte_bound_is_a_ceiling_on_a_batch_not_a_threshold(pool: PgPool) {
        let w = seed_world(&pool, "byte-ceiling-reconcile").await;
        backlog_of(&pool, &w, 2).await;
        // Both artifacts hold the same content, so one byte past one artifact admits one, and a
        // threshold reading of the bound would admit both.
        let one: i64 = sqlx::query_scalar("SELECT max(content_bytes) FROM kb_data_artifacts")
            .fetch_one(&pool)
            .await
            .unwrap();

        let summary = dispatch_tick_inner(
            &pool,
            None,
            ROOMY,
            BatchBound {
                rows: 100,
                bytes: one + 1,
            },
        )
        .await
        .unwrap();

        assert_eq!(summary.verdicts_written, 2);
        assert_eq!(
            summary.batches_read, 2,
            "the second artifact would take the batch past its bound, so it starts the next one"
        );
    }

    /// FAILS IF: a job claimed after the deadline is spent is worked on anyway, or is dropped
    /// rather than handed back for the next pass.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_spent_deadline_hands_every_claimed_job_back_untouched(pool: PgPool) {
        let w = seed_world(&pool, "deferred-reconcile").await;
        backlog_of(&pool, &w, 1).await;

        let deferred = dispatch_tick_inner(
            &pool,
            None,
            std::time::Duration::ZERO,
            SHAPE_RECONCILE_BATCH,
        )
        .await
        .unwrap();
        assert_eq!(deferred.deferred, 1, "the one claimed job is handed back");
        assert_eq!(
            deferred.verdicts_written, 0,
            "no work is attempted past the deadline"
        );
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["done".to_owned(), "pending".to_owned()],
            "the claimed job is done and its successor waits for the next pass"
        );

        let next = dispatch_tick(&pool, None).await.unwrap();
        assert_eq!(
            next.verdicts_written, 1,
            "the next pass reconciles the anchor"
        );
        assert_eq!(next.completed, 1);
    }

    /// FAILS IF: an anchor cut off by the deadline mid-backlog loses the verdicts it wrote, or its
    /// job is marked done with the rest of the backlog unreached.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_anchor_cut_off_mid_backlog_keeps_its_verdicts_and_resumes(pool: PgPool) {
        let w = seed_world(&pool, "partial-reconcile").await;
        backlog_of(&pool, &w, 5).await;

        let claimed = workflow_job_service::claim_anchor(
            &pool,
            Persona::Shape.as_str(),
            DispatchType::ShapeReconcile.as_str(),
            1,
            DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS,
        )
        .await
        .unwrap();
        let job = claimed.first().expect("the declare queued a job");

        // A deadline spent as soon as the pass starts. The job was not past it when it came up
        // (`past_deadline` false), so the pass reaches its first artifact, and the deadline is
        // spent at the check after it.
        let pace = Pace {
            start: std::time::Instant::now(),
            deadline: std::time::Duration::ZERO,
            batch: BatchBound {
                rows: 2,
                bytes: i64::MAX,
            },
        };
        let ran = run_reconcile_job(&pool, job, None, false, pace)
            .await
            .unwrap();
        let ReconcileJobResult::Ran { pass, settled } = ran else {
            panic!("the job ran a pass: {ran:?}");
        };
        assert!(
            !pass.finished,
            "the deadline came before the backlog was exhausted"
        );
        assert_eq!(settled, Settled::HandedBack);
        assert_eq!(pass.verdicts_written, 1, "one artifact was verdicted");
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["done".to_owned(), "pending".to_owned()],
            "a partial anchor is handed back to resume"
        );

        let rest = dispatch_tick(&pool, None).await.unwrap();
        assert_eq!(
            rest.verdicts_written, 4,
            "the next pass verdicts only what is left"
        );
        assert_eq!(rest.completed, 1);
    }

    /// FAILS IF: an artifact committed with `null` content, which has no content row, fails its
    /// anchor's pass instead of being verdicted as the commit verdicted it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_artifact_with_null_content_is_verdicted_not_failed(pool: PgPool) {
        let w = seed_world(&pool, "null-content-reconcile").await;
        let resource = ResourceId::from(w.resource);
        let (artifact, _) = commit_artifact(
            &pool,
            resource,
            "measurement",
            &serde_json::Value::Null,
            w.emitter,
        )
        .await;
        let has_content_row: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM kb_data_artifact_content WHERE artifact_id = $1)",
        )
        .bind(artifact.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            !has_content_row,
            "precondition: null content stores no content row"
        );
        declare_shape(
            &pool,
            DeclareShapeServiceParams {
                home: AnchorRef::context(ContextId::from(w.context)),
                kind: "measurement",
                kind_owner: None,
                schema: &string_value_schema(),
                enforcement: EnforcementMode::Advisory,
                principal: ProfileId::from(w.owner),
                emitter: EntityId::from(w.emitter),
            },
        )
        .await
        .unwrap();

        let summary = dispatch_tick(&pool, None).await.unwrap();

        assert_eq!(summary.failed, 0, "the anchor's pass does not fail");
        assert_eq!(summary.verdicts_written, 1);
        assert_eq!(
            read_shape_state(&pool, ProfileId::from(w.owner), resource).await,
            Some(ShapeState::DeclaredNotSatisfied),
            "null is validated against the shape as the commit validates it: an object is required"
        );
    }

    /// FAILS IF: a change that coalesced into a running job is lost when the job completes, so its
    /// artifacts never gain a verdict for the shape now in force.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_change_that_coalesced_into_a_running_job_is_queued_again_when_it_completes(
        pool: PgPool,
    ) {
        let w = seed_world(&pool, "coalesced-reconcile").await;
        backlog_of(&pool, &w, 2).await;
        let claimed = workflow_job_service::claim_anchor(
            &pool,
            Persona::Shape.as_str(),
            DispatchType::ShapeReconcile.as_str(),
            1,
            DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS,
        )
        .await
        .unwrap();
        let job = claimed.first().expect("the declare queued a job");

        // The pass verdicts the backlog against version 1.
        let pace = Pace {
            start: std::time::Instant::now(),
            deadline: ROOMY,
            batch: SHAPE_RECONCILE_BATCH,
        };
        let pass = reconcile_one_anchor(&pool, job.anchor, pace).await.unwrap();
        assert_eq!(pass.verdicts_written, 2);

        // An amendment lands while the job is still in flight: its enqueue coalesces into it.
        let mut amended = string_value_schema();
        amended["properties"]["unit"] = serde_json::json!({ "type": "string" });
        declare_shape(
            &pool,
            DeclareShapeServiceParams {
                home: AnchorRef::context(ContextId::from(w.context)),
                kind: "measurement",
                kind_owner: None,
                schema: &amended,
                enforcement: EnforcementMode::Advisory,
                principal: ProfileId::from(w.owner),
                emitter: EntityId::from(w.emitter),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["in_progress".to_owned()],
            "precondition: the amendment's enqueue coalesced into the running job"
        );

        let settled = settle(&pool, job, &pass).await.unwrap();

        assert_eq!(settled, Settled::Completed { requeued: true });
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["done".to_owned(), "pending".to_owned()],
            "the job is done, and a successor waits for the amended shape's backlog"
        );
        let next = dispatch_tick(&pool, None).await.unwrap();
        assert_eq!(
            next.verdicts_written, 2,
            "the next pass verdicts both artifacts against the amendment"
        );
    }

    /// FAILS IF: an anchor queued again after completing is run a second time by the same pass,
    /// so a backlog the pass cannot clear spins it to the deadline.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_pass_runs_each_anchor_once_and_leaves_a_requeued_one_for_the_next(pool: PgPool) {
        let w = seed_world(&pool, "requeued-once-reconcile").await;
        backlog_of(&pool, &w, 1).await;
        // A backlog the pass leaves behind every time: the verdict write never lands, and the job
        // is settled as if it had, so `settle` queues it again.
        sqlx::query(
            "CREATE FUNCTION drop_every_verdict() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN RETURN NULL; END $$",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER drop_every_verdict BEFORE INSERT ON kb_data_artifact_verdicts \
             FOR EACH ROW EXECUTE FUNCTION drop_every_verdict()",
        )
        .execute(&pool)
        .await
        .unwrap();

        let summary = dispatch_tick_inner(&pool, None, ROOMY, SHAPE_RECONCILE_BATCH)
            .await
            .unwrap();

        assert_eq!(summary.completed, 1, "the anchor ran once");
        assert_eq!(
            summary.requeued, 1,
            "its backlog was not empty, so it was queued again"
        );
        assert_eq!(
            summary.deferred, 1,
            "the pass claimed it again and handed it back untouched rather than running it twice"
        );
        assert_eq!(
            reconcile_job_statuses(&pool, w.context)
                .await
                .last()
                .map(String::as_str),
            Some("pending"),
            "the anchor waits for the next pass"
        );
    }

    /// FAILS IF: one artifact whose verdict cannot be written stops its anchor's pass, so the
    /// artifacts after it are never reached; or the job is marked done with that artifact unchecked.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_artifact_whose_verdict_cannot_be_written_holds_back_nothing_else(pool: PgPool) {
        let w = seed_world(&pool, "poisoned-reconcile").await;
        let artifacts = backlog_of(&pool, &w, 3).await;
        let mut ordered: Vec<Uuid> = artifacts.iter().map(|a| a.uuid()).collect();
        ordered.sort();
        let poisoned = ordered[0];

        // A fault on the verdict write for the first artifact the pass reaches, in this test's own
        // throwaway database.
        sqlx::query(&format!(
            "CREATE FUNCTION refuse_one_verdict() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN IF NEW.artifact_id = '{poisoned}' THEN \
               RAISE EXCEPTION 'injected verdict fault'; END IF; RETURN NEW; END $$"
        ))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER refuse_one_verdict BEFORE INSERT ON kb_data_artifact_verdicts \
             FOR EACH ROW EXECUTE FUNCTION refuse_one_verdict()",
        )
        .execute(&pool)
        .await
        .unwrap();

        let summary = dispatch_tick(&pool, None).await.unwrap();

        assert_eq!(
            summary.verdicts_written, 2,
            "the artifacts after the failing one are verdicted"
        );
        assert_eq!(summary.failed_artifacts, 1);
        assert_eq!(summary.failed, 1, "the job is counted as failed");
        assert_eq!(summary.completed, 0);
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["in_progress".to_owned()],
            "the job is left in flight for the reaper, not marked done"
        );
        assert_eq!(
            verdict_version(&pool, DataArtifactId::from(poisoned)).await,
            None
        );
    }

    /// FAILS IF: a caller's `cap` reaches the claim unbounded, or a default other than one anchor
    /// at a time is used.
    #[test]
    fn a_claim_takes_one_anchor_by_default_and_never_more_than_the_ceiling() {
        assert_eq!(resolve_cap(None), 1);
        assert_eq!(resolve_cap(Some(4)), 4);
        assert_eq!(resolve_cap(Some(i32::MAX)), MAX_SHAPE_RECONCILE_CAP);
        assert_eq!(
            resolve_cap(Some(-3)),
            1,
            "the service holds even a caller past the door"
        );
    }
}
