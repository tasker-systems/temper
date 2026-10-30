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
use temper_core::types::data_artifact::{overlong_kind, unstorable_nul};
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
/// sentence naming the bound; one within it that cannot be compiled (malformed, not a field
/// tree, or with a pattern past `shape_schema::MAX_PATTERN_BYTES`) is
/// refused with the no-fetch constructor's fixed sentence. Both are a `DataArtifactRefusal`, so
/// a schema no ask could use is refused when it is declared rather than stored. Before either, a
/// family name or a schema holding U+0000, which Postgres cannot store, is refused as one too,
/// naming the field ([`unstorable_nul`]), where binding it would have failed in the database; and
/// so is a family name longer than [`MAX_ARTIFACT_KIND_BYTES`], naming its limit
/// ([`overlong_kind`]).
///
/// [`MAX_ARTIFACT_KIND_BYTES`]: temper_core::types::data_artifact::MAX_ARTIFACT_KIND_BYTES
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
    if let Some(refusal) =
        unstorable_nul(p.kind, "schema", p.schema).or_else(|| overlong_kind(p.kind))
    {
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
    /// Anchors whose whole backlog was settled, and whose job was marked done.
    pub completed: u32,
    /// Jobs handed back untouched: the invocation's deadline was already spent when they came up,
    /// or their anchor had already run in this pass.
    pub deferred: u32,
    /// Anchors that settled part of their backlog before their slice or the deadline ran out;
    /// re-enqueued to resume.
    pub partial: u32,
    /// Jobs left in flight for the reaper to retry: the pass errored, an artifact could not be
    /// settled, or the job could not be settled itself.
    pub failed: u32,
    /// Anchors re-enqueued after completing because their backlog was not empty: a declare, an
    /// amendment or a rehome that arrived while the job ran, and coalesced into it.
    pub requeued: u32,
    /// Verdicts written this invocation: the work product, which a tick that claimed nothing must
    /// not be mistaken for.
    pub verdicts_written: u64,
    /// Shapes found this invocation not to compile, and recorded in
    /// `kb_shape_reconcile_uncheckable` so no later pass selects their artifacts again.
    pub shapes_uncheckable: u32,
    /// Artifacts passed over this invocation because their shape in force does not compile. They
    /// stay `declared_not_yet_checked`.
    pub uncheckable_artifacts: u32,
    /// Artifacts whose content this drain cannot read, recorded this invocation in
    /// `kb_shape_reconcile_unreadable`: past the one-artifact ceiling, or past what Postgres can
    /// render. They stay `declared_not_yet_checked`.
    pub unreadable_artifacts: u32,
    /// Artifacts that could not be settled for a reason that may pass: a measurement, a check or
    /// a verdict write that failed. Each is named in a warning, and its anchor is never left
    /// without a job while it remains: the job is handed back to a successor if the pass was cut
    /// off after settling something, and otherwise left in flight for the reaper, ending `dead` if
    /// the failure never passes.
    pub failed_artifacts: u32,
    /// Backlog batches read this invocation.
    pub batches_read: u32,
}

/// What one backlog batch may hold, so an anchor's backlog is never read into memory whole.
///
/// Content is measured as Postgres returns it: the text of the stored `jsonb`. That is not the
/// committed `content_bytes`, because `jsonb` prints numbers in full, so `[1e308]` comes back about
/// fifty times longer than it was sent.
#[derive(Debug, Clone, Copy)]
struct BatchBound {
    /// At most this many artifacts.
    rows: usize,
    /// At most this many bytes of returned content, with one exception: the first artifact of a
    /// batch is always read, however large, so a batch always moves forward.
    bytes: i64,
    /// No artifact whose returned content is larger than this is read at all: it is recorded as
    /// `content_too_large` ([`MAX_RETURNED_CONTENT_BYTES`] in production).
    max_one: i64,
}

/// The batch the production drain reads: at most 200 artifacts and 32 MB of returned content, or
/// one larger artifact alone.
const SHAPE_RECONCILE_BATCH: BatchBound = BatchBound {
    rows: 200,
    bytes: 32_000_000,
    max_one: MAX_RETURNED_CONTENT_BYTES,
};

/// The most returned content the drain reads for one artifact, in bytes. A commit is bounded by
/// the 25 MiB gated body limit, so only content that `jsonb` prints far longer than it was
/// committed can pass it. Such an artifact is recorded as `content_too_large` and stays
/// `declared_not_yet_checked`. The `api/internal` function has 3 GB, and parsing what one artifact
/// returns holds a few times its size.
const MAX_RETURNED_CONTENT_BYTES: i64 = 256_000_000;

/// How many backlog artifacts one query reads, on cheap columns only. Selecting the backlog
/// evaluates the shape in force for every artifact of the anchor after the cursor, whatever the
/// `LIMIT`, so the drain selects a page this large once and cuts its batches from it, rather than
/// selecting each batch.
const BACKLOG_PAGE_ROWS: i64 = 10_000;

/// How many backlog artifacts the probe after a pass counts before it stops. Past it, the job
/// span's `backlog_remaining` is this number and `backlog_remaining_capped` is true: at least this
/// many remain, not exactly this many.
const BACKLOG_PROBE_ROWS: i64 = 1_000;

/// How many artifacts in a row may fail one step before the pass gives up on the anchor. Only a
/// failure that may pass counts: one is an artifact's, a run of them is the database's, and walking
/// the rest of the backlog would only repeat it. Measuring and verdicting keep separate runs
/// ([`Step`]): a measurement breaks only a run of failed measurements, a verdict only a run of
/// failed verdicts, and a record (unreadable content, or a shape that does not compile) both.
const MAX_CONSECUTIVE_FAILED_ARTIFACTS: u32 = 8;

/// Wall-clock ceiling for one drain invocation, in seconds: the region drain's, for its reason. It
/// sits under the `api/internal` function's `maxDuration: 300`, so a pass that runs long still has
/// room to hand its job back and return a summary rather than be killed mid-batch. It is checked
/// after every verdict, record and failure, not after a measurement, so a pass overruns it by
/// about one batch measured and read (bounded by [`BatchBound::bytes`] plus the artifact that
/// overflows it, each artifact at most [`MAX_RETURNED_CONTENT_BYTES`]), one artifact's check and
/// write, and whatever ran unchecked between them: a backlog page selected, and shapes loaded or
/// compiled on first sight.
const DEFAULT_SHAPE_RECONCILE_DEADLINE_SECONDS: u64 = 240;

/// The most the env override may set the deadline to: 30 s under the `api/internal` function's
/// `maxDuration` (300, `vercel.json`), so a pass at the ceiling still has room to hand its job back
/// before Vercel kills it. The lease ([`DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS`]) is longer still.
const MAX_SHAPE_RECONCILE_DEADLINE_SECONDS: u64 = 270;

/// Env override for the deadline above.
const SHAPE_RECONCILE_DEADLINE_ENV: &str = "TEMPER_SHAPE_RECONCILE_DEADLINE_SECONDS";

/// The most of a pass one anchor's job may use before it is handed back to the end of the queue.
/// With one anchor claimed at a time, an anchor with a large backlog would otherwise hold the whole
/// deadline, and anchors queued behind it would wait a pass each. A slice leaves room for about
/// four anchors a pass, so an anchor queued `k`-th has its next slice within `ceil(k / 4)` passes.
/// An anchor handed back for its slice may run again in the same pass if no other is waiting and
/// it settled something; one that settled nothing is left for the reaper ([`settle`]).
const SHAPE_RECONCILE_ANCHOR_SLICE: std::time::Duration = std::time::Duration::from_secs(60);

fn resolve_dispatch_deadline() -> std::time::Duration {
    deadline_from(std::env::var(SHAPE_RECONCILE_DEADLINE_ENV).ok().as_deref())
}

/// The deadline an override sets: a positive whole number of seconds, held to
/// [`MAX_SHAPE_RECONCILE_DEADLINE_SECONDS`]; anything else is the default.
fn deadline_from(raw: Option<&str>) -> std::time::Duration {
    let secs = raw
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_SHAPE_RECONCILE_DEADLINE_SECONDS)
        .min(MAX_SHAPE_RECONCILE_DEADLINE_SECONDS);
    std::time::Duration::from_secs(secs)
}

/// The claim size a pass uses: the caller's, held to `1..=MAX_SHAPE_RECONCILE_CAP`, or the
/// default. The door refuses a `cap` below 1 before it reaches here. A cap above 1 brings back the
/// starvation [`DEFAULT_SHAPE_RECONCILE_CAP`] avoids; it is an operator's tool for draining a
/// backlog of small anchors faster, never the cron's.
fn resolve_cap(cap: Option<i32>) -> i32 {
    cap.unwrap_or(DEFAULT_SHAPE_RECONCILE_CAP)
        .clamp(1, MAX_SHAPE_RECONCILE_CAP)
}

/// Run one shape-reconcile drain pass: reap expired leases, then claim anchor-keyed jobs and
/// settle each anchor's backlog against the shape in force, in bounded batches, until the queue
/// is empty or the deadline is spent.
///
/// A pass that claims nothing writes nothing: assert on `verdicts_written`, not on `Ok`.
pub async fn dispatch_tick(pool: &PgPool, cap: Option<i32>) -> ApiResult<ShapeReconcileSummary> {
    let pace = Pace::new(
        std::time::Instant::now(),
        resolve_dispatch_deadline(),
        SHAPE_RECONCILE_BATCH,
    );
    dispatch_tick_inner(pool, cap, pace).await
}

/// [`dispatch_tick`] with its pace injected, so tests can force the deferral, slice and resumption
/// paths deterministically (`Duration::ZERO` defers every claimed job) without seeding a
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
        shapes_uncheckable = tracing::field::Empty,
        uncheckable_artifacts = tracing::field::Empty,
        unreadable_artifacts = tracing::field::Empty,
        failed_artifacts = tracing::field::Empty,
        batches_read = tracing::field::Empty,
    )
)]
async fn dispatch_tick_inner(
    pool: &PgPool,
    cap: Option<i32>,
    pace: Pace,
) -> ApiResult<ShapeReconcileSummary> {
    let persona = Persona::Shape.as_str();
    let dispatch = DispatchType::ShapeReconcile.as_str();

    // Reap stale leases before claiming, mirroring the region and embed drains.
    workflow_job_service::reap(pool, "shape reconcile lease expired").await?;

    let cap = resolve_cap(cap);
    let mut summary = ShapeReconcileSummary::default();

    // Before the first claim, for `drain_span::read_queue_depth`'s reason.
    let span = tracing::Span::current();
    let depth = drain_span::read_queue_depth(pool, persona, dispatch).await?;
    span.record("backlog_depth", depth.backlog_depth);
    if let Some(age) = depth.oldest_pending_age_ms {
        span.record("oldest_pending_age_ms", age);
    }

    // The region drain's do-while: always one claim, then keep claiming until the queue is empty or
    // the deadline is spent. A job goes back into the queue in four ways. One handed back because
    // the deadline is spent ends the loop. One handed back because its slice ran out made progress,
    // and may be claimed and run again. A completed anchor queued again (`settle`), when this pass
    // claims it again, is handed back untouched and ends the loop, so a backlog the pass cannot
    // clear can never spin it; so does a job the pass could not settle.
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
            let already_ran = ran_again || !ran_this_pass.insert(job.anchor.uuid());
            ran_again |= already_ran;
            let ran = run_reconcile_job(
                pool,
                &job,
                waits.get(&job.id).copied(),
                pace.spent() || already_ran,
                pace,
            )
            .await;
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
                        Settled::HandedBack => {
                            summary.partial += 1;
                            // An anchor its slice cut off may run again this pass, once it has
                            // made progress; one that made none would only spin.
                            if pass.stop == Stop::Slice && pass.made_progress() {
                                ran_this_pass.remove(&job.anchor.uuid());
                            }
                        }
                        Settled::LeftInFlight => summary.failed += 1,
                    }
                    summary.verdicts_written += pass.verdicts_written;
                    summary.shapes_uncheckable += pass.shapes_uncheckable;
                    summary.uncheckable_artifacts += pass.uncheckable_artifacts;
                    summary.unreadable_artifacts += pass.unreadable_artifacts;
                    summary.failed_artifacts += pass.failed_artifacts;
                    summary.batches_read += pass.batches_read;
                }
            }
        }

        if ran_again || pace.spent() {
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
    span.record("shapes_uncheckable", summary.shapes_uncheckable);
    span.record("uncheckable_artifacts", summary.uncheckable_artifacts);
    span.record("unreadable_artifacts", summary.unreadable_artifacts);
    span.record("failed_artifacts", summary.failed_artifacts);
    span.record("batches_read", summary.batches_read);
    Ok(summary)
}

/// The pass's clock, its per-anchor slice and its batch bound, carried to each anchor.
#[derive(Debug, Clone, Copy)]
struct Pace {
    start: std::time::Instant,
    deadline: std::time::Duration,
    /// [`SHAPE_RECONCILE_ANCHOR_SLICE`] in production.
    slice: std::time::Duration,
    batch: BatchBound,
}

impl Pace {
    fn new(start: std::time::Instant, deadline: std::time::Duration, batch: BatchBound) -> Self {
        Pace {
            start,
            deadline,
            slice: SHAPE_RECONCILE_ANCHOR_SLICE,
            batch,
        }
    }

    fn spent(&self) -> bool {
        self.start.elapsed() >= self.deadline
    }

    /// Why an anchor that began at `anchor_start` must stop now, if it must.
    fn stop_due(&self, anchor_start: std::time::Instant) -> Option<Stop> {
        if self.spent() {
            Some(Stop::Deadline)
        } else if anchor_start.elapsed() >= self.slice {
            Some(Stop::Slice)
        } else {
            None
        }
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
    /// The slice or the deadline ran out first: done, and a successor queued to resume.
    HandedBack,
    /// Left in flight for the reaper to retry, which finds only what is left: an artifact could not
    /// be settled, or the job itself could not be.
    LeftInFlight,
}

/// Why an anchor's pass stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The backlog was exhausted.
    Exhausted,
    /// The anchor used its slice ([`SHAPE_RECONCILE_ANCHOR_SLICE`]). What was settled stands; if
    /// anything was, the re-enqueued job resumes from what is left, behind the anchors already
    /// waiting ([`settle`]).
    Slice,
    /// The pass's deadline came first. As for a slice, and the pass ends.
    Deadline,
    /// [`MAX_CONSECUTIVE_FAILED_ARTIFACTS`] artifacts in a row could not be settled.
    Breaker,
}

/// What one anchor's pass did.
#[derive(Debug)]
struct AnchorPass {
    verdicts_written: u64,
    shapes_uncheckable: u32,
    uncheckable_artifacts: u32,
    unreadable_artifacts: u32,
    failed_artifacts: u32,
    /// Measurements that failed in a row, with no successful measurement or record between them.
    measure_failures_in_a_row: u32,
    /// Checks or verdict writes that failed in a row, with no verdict or record between them.
    verdict_failures_in_a_row: u32,
    batches_read: u32,
    stop: Stop,
}

/// Which step of settling an artifact failed: each keeps its own run for the breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Measuring its content ([`returned_bytes`]).
    Measure,
    /// Checking it or writing its verdict ([`verdict_batch`]). A failure to read a batch's content
    /// fails the whole anchor instead.
    Verdict,
}

impl AnchorPass {
    fn new() -> Self {
        AnchorPass {
            verdicts_written: 0,
            shapes_uncheckable: 0,
            uncheckable_artifacts: 0,
            unreadable_artifacts: 0,
            failed_artifacts: 0,
            measure_failures_in_a_row: 0,
            verdict_failures_in_a_row: 0,
            batches_read: 0,
            stop: Stop::Exhausted,
        }
    }

    /// Whether the pass settled any artifact: a verdict, or a record of why there is none.
    fn made_progress(&self) -> bool {
        self.verdicts_written > 0 || self.unreadable_artifacts > 0 || self.uncheckable_artifacts > 0
    }

    /// `step` succeeded for an artifact: that step's run of failures is broken.
    fn succeeded(&mut self, step: Step) {
        match step {
            Step::Measure => self.measure_failures_in_a_row = 0,
            Step::Verdict => self.verdict_failures_in_a_row = 0,
        }
    }

    /// An artifact was settled by a record rather than a verdict: both runs are broken.
    fn recorded(&mut self) {
        self.measure_failures_in_a_row = 0;
        self.verdict_failures_in_a_row = 0;
    }

    /// Count an artifact whose `step` failed for a reason that may pass, naming it in a warning.
    /// `Some(Stop::Breaker)` when it makes [`MAX_CONSECUTIVE_FAILED_ARTIFACTS`] failures of that
    /// step in a row.
    fn failed(
        &mut self,
        step: Step,
        anchor: HomeAnchor,
        artifact_id: Uuid,
        why: &dyn std::fmt::Display,
    ) -> Option<Stop> {
        tracing::warn!(
            error = %why,
            artifact_id = %artifact_id,
            anchor = %anchor.uuid(),
            "shape reconciliation could not settle one artifact; the pass goes on, and the \
             anchor keeps a job while the artifact remains"
        );
        self.failed_artifacts += 1;
        let run = match step {
            Step::Measure => &mut self.measure_failures_in_a_row,
            Step::Verdict => &mut self.verdict_failures_in_a_row,
        };
        *run += 1;
        (*run >= MAX_CONSECUTIVE_FAILED_ARTIFACTS).then_some(Stop::Breaker)
    }
}

/// Why the drain cannot check any artifact under a shape: the `reason` vocabulary of
/// `kb_shape_reconcile_uncheckable` (20261108000000).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Uncheckable {
    /// The shape does not compile ([`shape_schema::compile`]): a shape stored before compile
    /// refused anything but a field tree.
    SchemaUncompilable,
}

impl Uncheckable {
    fn as_str(self) -> &'static str {
        match self {
            Uncheckable::SchemaUncompilable => "schema_uncompilable",
        }
    }
}

/// Why the drain cannot read one artifact's content, whatever its shape: the `reason` vocabulary of
/// `kb_shape_reconcile_unreadable` (20261108000000). Each is a property of the stored content
/// under this drain's ceiling and Postgres's limits, so a pass that finds one would find it again.
/// A release that raises [`MAX_RETURNED_CONTENT_BYTES`] leaves `content_too_large` records standing
/// until the artifact or its shape changes; the operator's remedy is the migration's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unreadable {
    /// The content returns more than [`BatchBound::max_one`] bytes.
    ContentTooLarge,
    /// Postgres cannot render the content as text at all: measuring it raised a program-limit
    /// error (SQLSTATE class `54`), such as a string past 1 GB.
    ContentUnrenderable,
}

impl Unreadable {
    fn as_str(self) -> &'static str {
        match self {
            Unreadable::ContentTooLarge => "content_too_large",
            Unreadable::ContentUnrenderable => "content_unrenderable",
        }
    }
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
        uncheckable_artifacts = tracing::field::Empty,
        unreadable_artifacts = tracing::field::Empty,
        failed_artifacts = tracing::field::Empty,
        backlog_remaining = tracing::field::Empty,
        backlog_remaining_capped = tracing::field::Empty,
    )
)]
async fn run_reconcile_job(
    pool: &PgPool,
    job: &ClaimedAnchorJob,
    queue_wait_ms: Option<i64>,
    hand_back_untouched: bool,
    pace: Pace,
) -> ReconcileJobResult {
    let span = tracing::Span::current();
    if let Some(wait) = queue_wait_ms {
        span.record("queue_wait_ms", wait);
    }

    if hand_back_untouched {
        // Hand the job back untouched. Complete-then-re-enqueue rather than holding the lease, so
        // the reaper's attempt count stays clean — the region drain's reasoning.
        if let Err(e) = hand_back(pool, job).await {
            span.record("outcome", JobOutcome::Failed.as_str());
            tracing::warn!(
                error = %e,
                "could not hand an untouched shape-reconcile job back; left in flight for the reaper"
            );
            return ReconcileJobResult::Failed;
        }
        span.record("outcome", JobOutcome::Deferred.as_str());
        return ReconcileJobResult::Deferred;
    }

    match reconcile_one_anchor(pool, job.anchor, pace).await {
        Ok(pass) => {
            let settled = settle(pool, job, &pass).await;
            span.record(
                "outcome",
                match settled {
                    Settled::Completed { .. } => JobOutcome::Completed.as_str(),
                    Settled::HandedBack => JobOutcome::Partial.as_str(),
                    Settled::LeftInFlight => JobOutcome::Failed.as_str(),
                },
            );
            span.record("verdicts_written", pass.verdicts_written);
            span.record("uncheckable_artifacts", pass.uncheckable_artifacts);
            span.record("unreadable_artifacts", pass.unreadable_artifacts);
            span.record("failed_artifacts", pass.failed_artifacts);
            ReconcileJobResult::Ran { pass, settled }
        }
        Err(e) => {
            // Leave the job in_progress; the reaper retries it. One bad anchor never aborts the
            // pass — the region drain's posture.
            span.record("outcome", JobOutcome::Failed.as_str());
            tracing::warn!(
                error = %e,
                "shape reconciliation failed for one anchor; left in-flight for the reaper"
            );
            record_backlog_remaining(pool, job.anchor).await;
            ReconcileJobResult::Failed
        }
    }
}

/// Settle a job after its anchor ran a pass, and record on the job span how much backlog remains.
/// Never fails the pass: a job that cannot be settled is left in flight, with a warning, for the
/// reaper.
///
/// - A pass its slice or the deadline cut off hands the job back to resume, failed artifacts or
///   not, if it settled anything: the next pass finds the failures again, and a hand-back costs no
///   attempt. One that settled nothing is left in flight, so its attempt counts.
/// - A pass the breaker stopped, or one that exhausted the backlog with an artifact it could not
///   settle, leaves the job in flight, so the reaper retries it and counts the attempt: the retry
///   finds only what this pass did not settle. A failure that
///   does not pass ends the job `dead` after the reaper's attempts; the artifact stays in the
///   backlog for the next declare or rehome to reach.
/// - A clean, exhausted pass marks the job done, and then probes the backlog. The single-flight
///   index covers `in_progress`, so a declare, an amendment or a rehome that arrived while the job
///   ran coalesced into it, and its artifacts may sit behind the pass's cursor. Probed after the
///   job is done, a backlog that is not empty is queued again, and a later enqueue cannot coalesce
///   into the job just finished — the region drain's `requeue_if_erased_members` ordering. The
///   probe and the requeue are best-effort, as that one is: the job is already done. **Not
///   atomic:** a run killed between the completion and the requeue, which a full probe of the
///   anchor's backlog separates, loses the coalesced change until the next declare or rehome
///   reaches the anchor.
async fn settle(pool: &PgPool, job: &ClaimedAnchorJob, pass: &AnchorPass) -> Settled {
    let persona = Persona::Shape.as_str();
    let dispatch = DispatchType::ShapeReconcile.as_str();

    match pass.stop {
        Stop::Slice | Stop::Deadline if !pass.made_progress() => {
            // Cut off having settled nothing, so the stop came at a failure. A hand-back costs no
            // attempt, and the next pass could meet the same failure with nothing to stop it.
            // Left in flight, the reaper counts the attempt, and the job ends dead if it never
            // passes.
            tracing::warn!(
                failed_artifacts = pass.failed_artifacts,
                "a shape-reconcile pass ran out of time having settled nothing; its job is left in \
                 flight for the reaper, which counts the attempt"
            );
            record_backlog_remaining(pool, job.anchor).await;
            return Settled::LeftInFlight;
        }
        Stop::Slice | Stop::Deadline => {
            let settled = match hand_back(pool, job).await {
                Ok(()) => Settled::HandedBack,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "could not hand a partly reconciled anchor back; left in flight for the reaper"
                    );
                    Settled::LeftInFlight
                }
            };
            record_backlog_remaining(pool, job.anchor).await;
            return settled;
        }
        Stop::Breaker => {
            record_backlog_remaining(pool, job.anchor).await;
            return Settled::LeftInFlight;
        }
        Stop::Exhausted if pass.failed_artifacts > 0 => {
            record_backlog_remaining(pool, job.anchor).await;
            return Settled::LeftInFlight;
        }
        Stop::Exhausted => {}
    }
    if let Err(e) = workflow_job_service::complete_anchor(pool, job.anchor, persona, dispatch).await
    {
        tracing::warn!(
            error = %e,
            "could not mark a reconciled anchor's job done; left in flight for the reaper"
        );
        return Settled::LeftInFlight;
    }
    let Some(remaining) = record_backlog_remaining(pool, job.anchor).await else {
        return Settled::Completed { requeued: false };
    };
    if remaining == 0 {
        return Settled::Completed { requeued: false };
    }
    let requeued = workflow_job_service::enqueue_anchor(
        pool,
        job.anchor,
        persona,
        dispatch,
        AnchorJobPayload {
            emitter: job.emitter,
        },
    )
    .await;
    match requeued {
        Ok(_) => Settled::Completed { requeued: true },
        Err(e) => {
            tracing::warn!(
                error = %e,
                "could not queue a completed anchor again for its remaining backlog; it waits for \
                 the next declare or rehome to reach it"
            );
            Settled::Completed { requeued: false }
        }
    }
}

/// Probe how much backlog the anchor has left and record it on the current job span. Best-effort:
/// `None`, with a warning, when the probe fails.
async fn record_backlog_remaining(pool: &PgPool, anchor: HomeAnchor) -> Option<i64> {
    match backlog_page(pool, anchor, Uuid::nil(), BACKLOG_PROBE_ROWS + 1).await {
        Ok(rows) => {
            let remaining = rows.len() as i64;
            let span = tracing::Span::current();
            span.record("backlog_remaining", remaining.min(BACKLOG_PROBE_ROWS));
            span.record("backlog_remaining_capped", remaining > BACKLOG_PROBE_ROWS);
            Some(remaining)
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not probe the anchor's remaining backlog");
            None
        }
    }
}

/// Mark the job done and enqueue its successor, in one transaction, so the next pass resumes the
/// anchor. A failure rolls both back and leaves the job in flight for the reaper; it never leaves
/// the anchor with no job.
async fn hand_back(pool: &PgPool, job: &ClaimedAnchorJob) -> ApiResult<()> {
    let persona = Persona::Shape.as_str();
    let dispatch = DispatchType::ShapeReconcile.as_str();
    let mut tx = pool.begin().await?;
    workflow_job_service::complete_anchor(&mut *tx, job.anchor, persona, dispatch).await?;
    workflow_job_service::enqueue_anchor(
        &mut *tx,
        job.anchor,
        persona,
        dispatch,
        AnchorJobPayload {
            emitter: job.emitter,
        },
    )
    .await?;
    tx.commit().await?;
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

/// The anchor's backlog after `after`, in artifact-id order, at most `limit` artifacts.
///
/// The backlog is the live artifacts of active resources whose shape in force is homed at
/// `anchor`, whose staleness triple `(shape_id, shape_version, content_hash)` matches no stored
/// verdict and no unreadable record (`kb_shape_reconcile_unreadable`), and whose shape in force is
/// not recorded as uncheckable (`kb_shape_reconcile_uncheckable`).
///
/// - An erased resource's artifacts are not backlog: the act empties their content but leaves them
///   unfolded with their hash (D2 step 5), so a shape declared or amended after the act would
///   select them, and the verdict writer refuses them (`_resource_write_guard`, D13).
/// - Nor are an inactive resource's: `is_active` is the erasure estate's fence for a resource
///   (`policy/erasure/estate.toml`), and a fenced resource's content is not processed. One made
///   active again is backlog again, but nothing queues its anchor for that: it waits for the next
///   declare or rehome to reach the anchor.
///
/// Uses _data_artifact_shape_in_force (which calls _data_artifact_anchor and
/// _data_artifact_kind_owner) rather than re-deriving the home — the cogmap tiebreak is
/// load-bearing (20260820000020:186-188). The home-row EXISTS only narrows the scan to this
/// anchor's resources: the shape in force is homed at the home `_data_artifact_anchor` picks,
/// which is one of the resource's own, so no artifact the shape join admits is lost.
async fn backlog_page(
    pool: &PgPool,
    anchor: HomeAnchor,
    after: Uuid,
    limit: i64,
) -> ApiResult<Vec<Candidate>> {
    let rows = sqlx::query_as!(
        Candidate,
        r#"
        SELECT a.id            AS "artifact_id!: Uuid",
               a.content_hash  AS "content_hash!: String",
               s.shape_id      AS "shape_id!: Uuid",
               s.shape_version AS "shape_version!: i32"
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
           AND r.is_active
           AND a.id > $3
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
           AND NOT EXISTS (
               SELECT 1 FROM kb_shape_reconcile_uncheckable u
                WHERE u.shape_id = s.shape_id
           )
           AND NOT EXISTS (
               SELECT 1 FROM kb_shape_reconcile_unreadable x
                WHERE x.artifact_id   = a.id
                  AND x.shape_id      = s.shape_id
                  AND x.shape_version = s.shape_version
                  AND x.content_hash  = a.content_hash
           )
         ORDER BY a.id
         LIMIT $4
        "#,
        anchor.table(),
        anchor.uuid(),
        after,
        limit,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Whether an artifact returning `next` bytes joins a batch that already holds `len` artifacts and
/// `bytes` bytes: always when the batch is empty, so a batch always moves forward; otherwise only
/// within both of `bound`'s limits.
fn joins_batch(len: usize, bytes: i64, next: i64, bound: BatchBound) -> bool {
    len == 0 || (len < bound.rows && bytes.saturating_add(next) <= bound.bytes)
}

/// What the pass knows about one shape: compiled, or recorded as uncheckable.
enum ShapeCheck {
    Ready(Arc<shape_schema::Validator>),
    Uncheckable,
}

/// A batch the pass has measured and not yet read.
type Batch<'a> = Vec<(&'a Candidate, Arc<shape_schema::Validator>)>;

/// Settle one anchor's backlog, a bounded batch at a time, until it is exhausted, its slice or the
/// deadline runs out, or [`MAX_CONSECUTIVE_FAILED_ARTIFACTS`] artifacts in a row could not be
/// settled.
///
/// A backlog page is selected once on cheap columns ([`backlog_page`]). Its artifacts are measured
/// one at a time as Postgres returns them ([`returned_bytes`]), and gathered into a batch until the
/// next would take it past its bound ([`joins_batch`]); the batch is then read and verdicted. So the
/// content one batch reads, and the measuring done between two batches, is bounded by what
/// arrives, not by what was committed. The slice and the deadline are checked after every verdict,
/// record of unreadable content and failure, not after a measurement or a pass-over. A slice or
/// deadline stop reached while measuring first tries the first artifact measured since the last
/// batch, if any; a breaker stop does not.
///
/// - **A shape that does not compile is recorded once** ([`load_shape_check`]), and its artifacts
///   are passed over, here and by every later backlog selection.
/// - **An artifact whose content cannot be read is recorded** ([`Unreadable`]) under its staleness
///   triple, and passed over until it or its shape changes.
/// - **Compiles and checks run on the blocking pool** under the shape compile permits
///   ([`shape_loader::run_off_the_executor`]), not on the executor. A field tree bounds what one
///   check costs by its content, so no check is bounded or isolated here. A panic in either aborts
///   the process (`panic = "abort"` in the release profile), and the reaper retries the job.
/// - **A verdict names only the first failure** ([`shape_schema::failure_detail`]), as a commit's
///   does.
/// - **An artifact that cannot be settled for a reason that may pass** is named in a warning and
///   counted, and the pass goes on past it, until [`MAX_CONSECUTIVE_FAILED_ARTIFACTS`] of one step
///   fail in a row.
async fn reconcile_one_anchor(
    pool: &PgPool,
    anchor: HomeAnchor,
    pace: Pace,
) -> ApiResult<AnchorPass> {
    let anchor_start = std::time::Instant::now();
    let mut pass = AnchorPass::new();
    let mut after = Uuid::nil();
    let mut shapes: HashMap<Uuid, ShapeCheck> = HashMap::new();

    loop {
        let page = backlog_page(pool, anchor, after, BACKLOG_PAGE_ROWS).await?;
        let Some(last) = page.last() else {
            pass.stop = Stop::Exhausted;
            return Ok(pass);
        };
        after = last.artifact_id;

        let mut batch: Batch<'_> = Vec::new();
        let mut batch_bytes: i64 = 0;
        for c in &page {
            let validator = match load_shape_check(pool, anchor, &mut shapes, c, &mut pass).await? {
                ShapeCheck::Ready(validator) => Arc::clone(validator),
                ShapeCheck::Uncheckable => {
                    pass.uncheckable_artifacts += 1;
                    continue;
                }
            };
            let stop = match returned_bytes(pool, c.artifact_id).await {
                Ok(bytes) if bytes > pace.batch.max_one => {
                    record_unreadable(pool, c, Unreadable::ContentTooLarge, &mut pass).await?;
                    pace.stop_due(anchor_start)
                }
                Ok(bytes) => {
                    pass.succeeded(Step::Measure);
                    if !joins_batch(batch.len(), batch_bytes, bytes, pace.batch) {
                        let full = std::mem::take(&mut batch);
                        if let Some(stop) =
                            verdict_batch(pool, anchor, full, &mut pass, pace, anchor_start).await?
                        {
                            pass.stop = stop;
                            return Ok(pass);
                        }
                        batch_bytes = 0;
                    }
                    batch.push((c, validator));
                    batch_bytes = batch_bytes.saturating_add(bytes);
                    None
                }
                Err(e) if is_program_limit(&e) => {
                    record_unreadable(pool, c, Unreadable::ContentUnrenderable, &mut pass).await?;
                    pace.stop_due(anchor_start)
                }
                Err(e) => pass
                    .failed(Step::Measure, anchor, c.artifact_id, &e)
                    .or_else(|| pace.stop_due(anchor_start)),
            };
            if let Some(stop) = stop {
                // Try the first artifact this batch had gathered before stopping, so a stop that
                // comes after a failure can still settle something. Only the first: the stop is
                // already due, so `verdict_batch` would stop after it anyway.
                pass.stop = match stop {
                    Stop::Breaker => Stop::Breaker,
                    _ => {
                        batch.truncate(1);
                        verdict_batch(pool, anchor, batch, &mut pass, pace, anchor_start)
                            .await?
                            .unwrap_or(stop)
                    }
                };
                return Ok(pass);
            }
        }
        if let Some(stop) =
            verdict_batch(pool, anchor, batch, &mut pass, pace, anchor_start).await?
        {
            pass.stop = stop;
            return Ok(pass);
        }
    }
}

/// Read one measured batch's content and verdict each artifact. `Some` when the pass must stop:
/// the breaker tripped, or the slice or the deadline ran out.
async fn verdict_batch(
    pool: &PgPool,
    anchor: HomeAnchor,
    batch: Batch<'_>,
    pass: &mut AnchorPass,
    pace: Pace,
    anchor_start: std::time::Instant,
) -> ApiResult<Option<Stop>> {
    if batch.is_empty() {
        return Ok(None);
    }
    pass.batches_read += 1;
    let ids: Vec<Uuid> = batch.iter().map(|(c, _)| c.artifact_id).collect();
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

    for (c, validator) in batch {
        // An artifact committed with JSON `null` content has no content row
        // (20260820000020, `data_artifact_commit`), and the commit validated that `null` against
        // the shape, so reconciliation does the same.
        let content = contents
            .remove(&c.artifact_id)
            .unwrap_or(serde_json::Value::Null);
        match write_verdict(pool, c, validator, content).await {
            Ok(()) => {
                pass.verdicts_written += 1;
                pass.succeeded(Step::Verdict);
            }
            Err(e) => {
                if let Some(stop) = pass.failed(Step::Verdict, anchor, c.artifact_id, &e) {
                    return Ok(Some(stop));
                }
            }
        }
        if let Some(stop) = pace.stop_due(anchor_start) {
            return Ok(Some(stop));
        }
    }
    Ok(None)
}

/// What the pass knows about `c`'s shape, loaded once per shape. A shape row is never edited (an
/// amendment folds it and declares a new one), so what one pass learns of a shape holds for every
/// later pass under the same compiler: a shape that does not compile is recorded in
/// `kb_shape_reconcile_uncheckable`, named once in a warning, and never selected again.
async fn load_shape_check<'s>(
    pool: &PgPool,
    anchor: HomeAnchor,
    shapes: &'s mut HashMap<Uuid, ShapeCheck>,
    c: &Candidate,
    pass: &mut AnchorPass,
) -> ApiResult<&'s ShapeCheck> {
    let slot = match shapes.entry(c.shape_id) {
        Entry::Occupied(slot) => return Ok(slot.into_mut()),
        Entry::Vacant(slot) => slot,
    };
    let schema = sqlx::query_scalar!(
        r#"SELECT schema AS "schema!: serde_json::Value"
             FROM kb_data_artifact_shapes WHERE id = $1"#,
        c.shape_id,
    )
    .fetch_one(pool)
    .await?;
    let compiled =
        shape_loader::run_off_the_executor(move || shape_schema::compile(&schema).map(Arc::new))
            .await?;
    if let Ok(validator) = compiled {
        return Ok(slot.insert(ShapeCheck::Ready(validator)));
    }
    let reason = Uncheckable::SchemaUncompilable;
    let recorded = sqlx::query!(
        r#"
        INSERT INTO kb_shape_reconcile_uncheckable (shape_id, reason)
        VALUES ($1, $2)
        ON CONFLICT (shape_id) DO NOTHING
        "#,
        c.shape_id,
        reason.as_str(),
    )
    .execute(pool)
    .await?;
    if recorded.rows_affected() > 0 {
        pass.shapes_uncheckable += 1;
        pass.recorded();
    }
    tracing::warn!(
        shape_id = %c.shape_id,
        shape_version = c.shape_version,
        anchor = %anchor.uuid(),
        reason = reason.as_str(),
        "shape reconciliation cannot check any artifact under this shape; it is recorded as \
         uncheckable, and its artifacts stay declared_not_yet_checked"
    );
    Ok(slot.insert(ShapeCheck::Uncheckable))
}

/// Record that `c`'s content cannot be read, under its staleness triple, naming it once in a
/// warning. One row per artifact: a newer record replaces an older one.
async fn record_unreadable(
    pool: &PgPool,
    c: &Candidate,
    reason: Unreadable,
    pass: &mut AnchorPass,
) -> ApiResult<()> {
    sqlx::query!(
        r#"
        INSERT INTO kb_shape_reconcile_unreadable
               (artifact_id, shape_id, shape_version, content_hash, reason)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (artifact_id) DO UPDATE
           SET shape_id      = EXCLUDED.shape_id,
               shape_version = EXCLUDED.shape_version,
               content_hash  = EXCLUDED.content_hash,
               reason        = EXCLUDED.reason,
               recorded_at   = now()
        "#,
        c.artifact_id,
        c.shape_id,
        c.shape_version,
        c.content_hash,
        reason.as_str(),
    )
    .execute(pool)
    .await?;
    tracing::warn!(
        artifact_id = %c.artifact_id,
        shape_id = %c.shape_id,
        reason = reason.as_str(),
        "shape reconciliation cannot read this artifact's content; it is recorded as unreadable, \
         and stays declared_not_yet_checked"
    );
    pass.unreadable_artifacts += 1;
    pass.recorded();
    Ok(())
}

/// How many bytes Postgres returns for `artifact_id`'s content, by the one measure of the stored
/// form (`data_artifact_content_returned_bytes`, 20261108000000): the length of the stored `jsonb`'s
/// text, which is what a read receives. Zero for an artifact with no content row (committed as
/// `null`).
async fn returned_bytes(pool: &PgPool, artifact_id: Uuid) -> Result<i64, sqlx::Error> {
    let bytes = sqlx::query_scalar!(
        r#"SELECT data_artifact_content_returned_bytes($1) AS "bytes?: i64""#,
        artifact_id,
    )
    .fetch_one(pool)
    .await?;
    Ok(bytes.unwrap_or(0))
}

/// Whether `e` is Postgres refusing for a program limit (SQLSTATE class `54`): a property of what
/// was asked, which asking again will not change. Rendering content past 1 GB of text is one.
fn is_program_limit(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(db) => db.code().is_some_and(|code| code.starts_with("54")),
        _ => false,
    }
}

/// Check `content` against its shape on the blocking pool, and write the verdict.
async fn write_verdict(
    pool: &PgPool,
    c: &Candidate,
    validator: Arc<shape_schema::Validator>,
    content: serde_json::Value,
) -> ApiResult<()> {
    let failure = shape_loader::run_off_the_executor(move || {
        shape_schema::first_failure(&validator, &content)
    })
    .await?;
    let satisfied = failure.is_none();
    let detail = failure.map(|(path, message)| shape_schema::failure_detail(&path, &message));
    sqlx::query!(
        "SELECT data_artifact_verdict_upsert($1,$2,$3,$4,$5,$6)",
        c.artifact_id,
        c.shape_id,
        c.shape_version,
        c.content_hash,
        satisfied,
        detail,
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

    /// Reconciliation records ONE violation per artifact, not every one: content breaking three
    /// properties gets a stored detail with exactly one entry.
    #[sqlx::test(migrations = "../../migrations")]
    async fn reconcile_records_one_violation(pool: PgPool) {
        let w = seed_world(&pool, "reconcile-one-violation").await;
        let kind = "measurement";
        let resource = ResourceId::from(w.resource);

        let content = serde_json::json!({"alpha": "a", "bravo": "b", "charlie": "c"});
        let (artifact_id, _) = commit_artifact(&pool, resource, kind, &content, w.emitter).await;

        let schema = serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "alpha": { "type": "string", "minLength": 5 },
                "bravo": { "type": "string", "minLength": 5 },
                "charlie": { "type": "string", "minLength": 5 }
            },
            "required": ["alpha", "bravo", "charlie"]
        });
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

        let summary = dispatch_tick(&pool, None).await.unwrap();
        assert_eq!(
            summary.verdicts_written, 1,
            "the one pre-existing artifact is verdicted"
        );

        let detail: serde_json::Value = sqlx::query_scalar(
            "SELECT detail FROM kb_data_artifact_verdicts WHERE artifact_id = $1",
        )
        .bind(artifact_id.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            detail.as_array().map(Vec::len),
            Some(1),
            "the stored detail has exactly one entry, got: {detail}"
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
                &format!("backlog-{}-{i}", w.context),
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
            Pace::new(
                std::time::Instant::now(),
                ROOMY,
                BatchBound {
                    rows: 2,
                    bytes: i64::MAX,
                    max_one: i64::MAX,
                },
            ),
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
            Pace::new(
                std::time::Instant::now(),
                ROOMY,
                BatchBound {
                    rows: 100,
                    bytes: 1,
                    max_one: i64::MAX,
                },
            ),
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
        let one: i64 = sqlx::query_scalar(
            "SELECT max(octet_length(content::text))::bigint FROM kb_data_artifact_content",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let summary = dispatch_tick_inner(
            &pool,
            None,
            Pace::new(
                std::time::Instant::now(),
                ROOMY,
                BatchBound {
                    rows: 100,
                    bytes: one + 1,
                    max_one: i64::MAX,
                },
            ),
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
            Pace::new(
                std::time::Instant::now(),
                std::time::Duration::ZERO,
                SHAPE_RECONCILE_BATCH,
            ),
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
        let pace = Pace::new(
            std::time::Instant::now(),
            std::time::Duration::ZERO,
            BatchBound {
                rows: 2,
                bytes: i64::MAX,
                max_one: i64::MAX,
            },
        );
        let ran = run_reconcile_job(&pool, job, None, false, pace).await;
        let ReconcileJobResult::Ran { pass, settled } = ran else {
            panic!("the job ran a pass: {ran:?}");
        };
        assert_eq!(
            pass.stop,
            Stop::Deadline,
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
        let pace = Pace::new(std::time::Instant::now(), ROOMY, SHAPE_RECONCILE_BATCH);
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

        let settled = settle(&pool, job, &pass).await;

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

        // A short deadline, so the spin this test exists to catch fails it in seconds.
        let summary = dispatch_tick_inner(
            &pool,
            None,
            Pace::new(
                std::time::Instant::now(),
                std::time::Duration::from_secs(15),
                SHAPE_RECONCILE_BATCH,
            ),
        )
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

    /// The world's declare job, claimed, as the drain would hold it.
    async fn claimed_job(pool: &PgPool) -> ClaimedAnchorJob {
        workflow_job_service::claim_anchor(
            pool,
            Persona::Shape.as_str(),
            DispatchType::ShapeReconcile.as_str(),
            1,
            DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS,
        )
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("the declare queued a job")
    }

    /// Replace the world's live shape with one that is not a field tree, as a shape stored before
    /// compile refused anything else.
    async fn store_uncompilable_shape(pool: &PgPool, w: &World) {
        sqlx::query(
            "UPDATE kb_data_artifact_shapes SET schema = $1 \
              WHERE home_anchor_id = $2 AND NOT is_folded",
        )
        .bind(serde_json::json!({ "anyOf": [{ "type": "object" }] }))
        .bind(w.context)
        .execute(pool)
        .await
        .unwrap();
    }

    /// The reasons recorded in `kb_shape_reconcile_uncheckable`.
    async fn uncheckable_reasons(pool: &PgPool) -> Vec<String> {
        sqlx::query_scalar("SELECT reason FROM kb_shape_reconcile_uncheckable ORDER BY reason")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    /// The reason an artifact is recorded unreadable under, or `None`.
    async fn unreadable_reason(pool: &PgPool, artifact: Uuid) -> Option<String> {
        sqlx::query_scalar(
            "SELECT reason FROM kb_shape_reconcile_unreadable WHERE artifact_id = $1",
        )
        .bind(artifact)
        .fetch_optional(pool)
        .await
        .unwrap()
    }

    /// Fail every verdict write for `artifacts` (all of them when `None`), in this test's own
    /// throwaway database.
    async fn refuse_verdicts(pool: &PgPool, artifacts: Option<&[Uuid]>) {
        let condition = match artifacts {
            None => "true".to_owned(),
            Some(ids) => {
                let list: Vec<String> = ids.iter().map(|id| format!("'{id}'")).collect();
                format!("NEW.artifact_id IN ({})", list.join(","))
            }
        };
        sqlx::query(&format!(
            "CREATE FUNCTION refuse_verdicts() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN IF {condition} THEN RAISE EXCEPTION 'injected verdict fault'; END IF; \
             RETURN NEW; END $$"
        ))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER refuse_verdicts BEFORE INSERT ON kb_data_artifact_verdicts \
             FOR EACH ROW EXECUTE FUNCTION refuse_verdicts()",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// FAILS IF: a shape that no longer compiles fails its anchor, gains verdicts, or leaves its
    /// artifacts in the backlog, so every pass compiles it again and requeues the anchor.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_shape_that_does_not_compile_is_recorded_once_and_its_artifacts_leave_the_backlog(
        pool: PgPool,
    ) {
        let w = seed_world(&pool, "uncompilable-reconcile").await;
        let artifacts = backlog_of(&pool, &w, 2).await;
        store_uncompilable_shape(&pool, &w).await;

        let summary = dispatch_tick(&pool, None).await.unwrap();

        assert_eq!(summary.failed, 0, "the anchor's pass does not fail");
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.requeued, 0, "nothing is left to queue again");
        assert_eq!(summary.verdicts_written, 0);
        assert_eq!(summary.shapes_uncheckable, 1);
        assert_eq!(summary.uncheckable_artifacts, 2);
        assert_eq!(
            uncheckable_reasons(&pool).await,
            vec!["schema_uncompilable".to_owned()]
        );
        for artifact in &artifacts {
            assert_eq!(verdict_version(&pool, *artifact).await, None);
        }
        let anchor = HomeAnchor::Context(ContextId::from(w.context));
        assert!(
            backlog_page(&pool, anchor, Uuid::nil(), 10)
                .await
                .unwrap()
                .is_empty(),
            "an artifact under a recorded shape is not backlog"
        );
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["done".to_owned()]
        );
    }

    /// FAILS IF: a record outlives the shape it names, so an amendment of an uncompilable shape is
    /// never tried.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_amended_shape_is_tried_afresh(pool: PgPool) {
        let w = seed_world(&pool, "amended-uncompilable-reconcile").await;
        backlog_of(&pool, &w, 2).await;
        store_uncompilable_shape(&pool, &w).await;
        dispatch_tick(&pool, None).await.unwrap();

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

        assert_eq!(
            summary.verdicts_written, 2,
            "both artifacts are verdicted against the amendment"
        );
        assert_eq!(summary.shapes_uncheckable, 0);
    }

    /// FAILS IF: a batch is bounded by the bytes committed rather than the bytes Postgres returns.
    /// `jsonb` prints `1e308` as 309 digits, so two such artifacts that fit one batch by their
    /// committed size do not fit it by what is read.
    #[sqlx::test(migrations = "../../migrations")]
    async fn content_is_batched_by_what_postgres_returns_not_what_was_committed(pool: PgPool) {
        let w = seed_world(&pool, "returned-bytes-reconcile").await;
        let home = AnchorRef::context(ContextId::from(w.context));
        let content = serde_json::json!({ "value": 1e308 });
        for i in 0..2 {
            let resource = make_resource(
                &pool,
                ProfileId::from(w.owner),
                EntityId::from(w.emitter),
                home,
                &format!("wide-{i}"),
            )
            .await;
            commit_artifact(&pool, resource, "measurement", &content, w.emitter).await;
        }
        declare_shape(
            &pool,
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
        let bytes: i64 = 100;
        let (committed, returned): (i64, i64) = sqlx::query_as(
            "SELECT max(a.content_bytes), max(octet_length(c.content::text))::bigint \
               FROM kb_data_artifacts a JOIN kb_data_artifact_content c ON c.artifact_id = a.id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            2 * committed <= bytes && returned > bytes,
            "precondition: both fit the bound as committed ({committed} each), and neither as \
             returned ({returned} each)"
        );

        let summary = dispatch_tick_inner(
            &pool,
            None,
            Pace::new(
                std::time::Instant::now(),
                ROOMY,
                BatchBound {
                    rows: 100,
                    bytes,
                    max_one: i64::MAX,
                },
            ),
        )
        .await
        .unwrap();

        assert_eq!(summary.verdicts_written, 2);
        assert_eq!(
            summary.batches_read, 2,
            "each artifact returns more than the bound, so each is a batch alone"
        );
    }

    /// FAILS IF: an artifact whose returned content is past the per-artifact ceiling is read, is
    /// not recorded with its reason, stops the artifacts after it, or keeps its anchor's job alive.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_artifact_past_the_one_artifact_ceiling_is_recorded_unread_and_holds_back_nothing_else(
        pool: PgPool,
    ) {
        let w = seed_world(&pool, "ceiling-reconcile").await;
        let home = AnchorRef::context(ContextId::from(w.context));
        let mut wide = None;
        for (i, value) in ["42".to_owned(), "x".repeat(1_000), "43".to_owned()]
            .into_iter()
            .enumerate()
        {
            let resource = make_resource(
                &pool,
                ProfileId::from(w.owner),
                EntityId::from(w.emitter),
                home,
                &format!("ceiling-{i}"),
            )
            .await;
            let long = value.len() > 2;
            let (artifact, _) = commit_artifact(
                &pool,
                resource,
                "measurement",
                &serde_json::json!({ "value": value }),
                w.emitter,
            )
            .await;
            if long {
                wide = Some(artifact);
            }
        }
        declare_shape(
            &pool,
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

        let summary = dispatch_tick_inner(
            &pool,
            None,
            Pace::new(
                std::time::Instant::now(),
                ROOMY,
                BatchBound {
                    rows: 100,
                    bytes: i64::MAX,
                    max_one: 100,
                },
            ),
        )
        .await
        .unwrap();

        let wide = wide.expect("one artifact is wide");
        assert_eq!(summary.verdicts_written, 2, "the other two are verdicted");
        assert_eq!(summary.unreadable_artifacts, 1);
        assert_eq!(summary.failed_artifacts, 0, "a record is not a failure");
        assert_eq!(summary.completed, 1, "the job completes");
        assert_eq!(verdict_version(&pool, wide).await, None);
        assert_eq!(
            unreadable_reason(&pool, wide.uuid()).await.as_deref(),
            Some("content_too_large")
        );
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["done".to_owned()],
            "the recorded artifact is not backlog, so nothing is queued again"
        );
    }

    /// FAILS IF: a run of artifacts that cannot be written is walked to the end of the backlog, or
    /// stops the pass without leaving the job for the reaper.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_run_of_failed_artifacts_stops_the_pass_and_leaves_the_job_for_the_reaper(
        pool: PgPool,
    ) {
        let w = seed_world(&pool, "breaker-reconcile").await;
        backlog_of(&pool, &w, MAX_CONSECUTIVE_FAILED_ARTIFACTS as usize + 2).await;
        refuse_verdicts(&pool, None).await;

        let summary = dispatch_tick(&pool, None).await.unwrap();

        assert_eq!(
            summary.failed_artifacts, MAX_CONSECUTIVE_FAILED_ARTIFACTS,
            "the pass stops at the breaker, not at the end of the backlog"
        );
        assert_eq!(summary.verdicts_written, 0);
        assert_eq!(summary.failed, 1);
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["in_progress".to_owned()],
            "the job is left in flight for the reaper, which counts the attempt"
        );
    }

    /// FAILS IF: a pass cut off having settled nothing hands its job back, which costs no attempt,
    /// so an anchor whose failures use up its time is handed back every pass and never dies.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_pass_cut_off_having_settled_nothing_leaves_its_job_for_the_reaper(pool: PgPool) {
        let w = seed_world(&pool, "cut-off-failed-reconcile").await;
        let artifacts = backlog_of(&pool, &w, 3).await;
        let mut ordered: Vec<Uuid> = artifacts.iter().map(|a| a.uuid()).collect();
        ordered.sort();
        refuse_verdicts(&pool, Some(&ordered[..1])).await;
        let job = claimed_job(&pool).await;
        let pace = Pace::new(
            std::time::Instant::now(),
            std::time::Duration::ZERO,
            SHAPE_RECONCILE_BATCH,
        );

        let ran = run_reconcile_job(&pool, &job, None, false, pace).await;

        let ReconcileJobResult::Ran { pass, settled } = ran else {
            panic!("the job ran a pass: {ran:?}");
        };
        assert_eq!(pass.failed_artifacts, 1);
        assert!(!pass.made_progress());
        assert_eq!(pass.stop, Stop::Deadline);
        assert_eq!(settled, Settled::LeftInFlight);
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["in_progress".to_owned()],
            "left for the reaper, which counts the attempt"
        );
    }

    /// FAILS IF: a stop that comes after a failed measurement ends the pass without settling what
    /// it had measured, so the pass settles nothing and its job is left for the reaper.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_stop_after_a_failed_measurement_still_settles_what_was_measured(pool: PgPool) {
        let w = seed_world(&pool, "flush-on-stop-reconcile").await;
        let artifacts = backlog_of(&pool, &w, 3).await;
        let mut ordered: Vec<Uuid> = artifacts.iter().map(|a| a.uuid()).collect();
        ordered.sort();
        poison_measurement(&pool, &ordered[1..2], "XX000").await;
        let job = claimed_job(&pool).await;
        let pace = Pace::new(
            std::time::Instant::now(),
            std::time::Duration::ZERO,
            SHAPE_RECONCILE_BATCH,
        );

        let ran = run_reconcile_job(&pool, &job, None, false, pace).await;

        let ReconcileJobResult::Ran { pass, settled } = ran else {
            panic!("the job ran a pass: {ran:?}");
        };
        assert_eq!(pass.failed_artifacts, 1);
        assert_eq!(
            pass.verdicts_written, 1,
            "the artifact measured before the failure is settled"
        );
        assert_eq!(pass.stop, Stop::Deadline);
        assert_eq!(settled, Settled::HandedBack);
    }

    /// FAILS IF: a measurement breaks a run of failed verdict writes, so a database refusing every
    /// write trips the breaker only when eight artifacts share one batch.
    #[sqlx::test(migrations = "../../migrations")]
    async fn failed_verdict_writes_trip_the_breaker_across_batches(pool: PgPool) {
        let w = seed_world(&pool, "breaker-batches-reconcile").await;
        backlog_of(&pool, &w, MAX_CONSECUTIVE_FAILED_ARTIFACTS as usize + 2).await;
        refuse_verdicts(&pool, None).await;
        let one_a_batch = BatchBound {
            rows: 100,
            bytes: 1,
            max_one: i64::MAX,
        };

        let summary = dispatch_tick_inner(
            &pool,
            None,
            Pace::new(std::time::Instant::now(), ROOMY, one_a_batch),
        )
        .await
        .unwrap();

        assert_eq!(
            summary.failed_artifacts, MAX_CONSECUTIVE_FAILED_ARTIFACTS,
            "the breaker stops the pass at the eighth write, each in its own batch"
        );
        assert_eq!(summary.failed, 1);
    }

    /// FAILS IF: a hand-back whose successor cannot be queued still marks the job done, leaving the
    /// anchor with no job at all.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_hand_back_that_cannot_queue_its_successor_leaves_the_job_in_flight(pool: PgPool) {
        let w = seed_world(&pool, "atomic-hand-back").await;
        backlog_of(&pool, &w, 1).await;
        let job = claimed_job(&pool).await;
        sqlx::query(
            "CREATE FUNCTION refuse_jobs() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN RAISE EXCEPTION 'injected enqueue fault'; END $$",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER refuse_jobs BEFORE INSERT ON kb_workflow_jobs \
             FOR EACH ROW EXECUTE FUNCTION refuse_jobs()",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(hand_back(&pool, &job).await.is_err());
        assert_eq!(
            reconcile_job_statuses(&pool, w.context).await,
            vec!["in_progress".to_owned()],
            "the completion rolled back with the enqueue"
        );
    }

    /// FAILS IF: the deadline override is read past its ceiling, the ceiling leaves no room under
    /// the `api/internal` function's `maxDuration` to hand a job back, or a value that is not a
    /// positive whole number of seconds is used.
    #[test]
    fn the_deadline_override_is_held_under_the_function_max_duration() {
        let secs = |raw| deadline_from(raw).as_secs();
        assert_eq!(secs(None), DEFAULT_SHAPE_RECONCILE_DEADLINE_SECONDS);
        assert_eq!(secs(Some(" 60 ")), 60);
        assert_eq!(secs(Some("0")), DEFAULT_SHAPE_RECONCILE_DEADLINE_SECONDS);
        assert_eq!(secs(Some("soon")), DEFAULT_SHAPE_RECONCILE_DEADLINE_SECONDS);
        assert_eq!(secs(Some("100000")), MAX_SHAPE_RECONCILE_DEADLINE_SECONDS);

        let vercel: serde_json::Value =
            serde_json::from_str(include_str!("../../../../vercel.json")).unwrap();
        let max_duration = vercel["functions"]["api/internal.rs"]["maxDuration"]
            .as_u64()
            .expect("vercel.json names api/internal.rs's maxDuration");
        assert!(
            MAX_SHAPE_RECONCILE_DEADLINE_SECONDS + 30 <= max_duration,
            "the ceiling leaves 30 s under maxDuration ({max_duration}) to hand a job back"
        );
        assert!(
            max_duration < u64::try_from(DEFAULT_SHAPE_RECONCILE_LEASE_SECONDS).unwrap(),
            "a killed function's job is reaped only after its lease"
        );
    }

    /// FAILS IF: a batch takes an artifact that crosses its byte or row bound, or ever refuses
    /// the first artifact.
    #[test]
    fn an_artifact_joins_a_batch_only_within_its_bounds_and_the_first_always() {
        let bound = BatchBound {
            rows: 3,
            bytes: 40,
            max_one: i64::MAX,
        };
        assert!(joins_batch(0, 0, 500, bound), "the first always");
        assert!(joins_batch(2, 30, 10, bound), "exactly at the byte bound");
        assert!(!joins_batch(2, 30, 11, bound), "one byte past it");
        assert!(!joins_batch(3, 3, 1, bound), "past the row bound");
    }

    /// The contexts of the artifacts the drain verdicted, in the order it wrote them.
    async fn verdict_contexts_in_order(pool: &PgPool) -> Vec<Uuid> {
        sqlx::query_scalar(
            "SELECT h.anchor_id FROM kb_data_artifact_verdicts v \
               JOIN kb_data_artifacts a ON a.id = v.artifact_id \
               JOIN kb_resource_homes h ON h.resource_id = a.resource_id \
              ORDER BY v.checked_at",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// FAILS IF: an anchor holds the pass until its backlog is exhausted, so an anchor queued
    /// behind it waits; or one cut off by its slice is not run again when the pass has time.
    #[sqlx::test(migrations = "../../migrations")]
    async fn anchors_take_turns_by_slice_and_each_finishes_in_one_pass(pool: PgPool) {
        let first = seed_world(&pool, "slice-first").await;
        backlog_of(&pool, &first, 3).await;
        let second = World {
            context: insert_context(&pool, first.owner, "slice-second").await,
            ..first
        };
        backlog_of(&pool, &second, 3).await;
        let pace = Pace {
            slice: std::time::Duration::ZERO,
            ..Pace::new(std::time::Instant::now(), ROOMY, SHAPE_RECONCILE_BATCH)
        };

        let summary = dispatch_tick_inner(&pool, None, pace).await.unwrap();

        assert_eq!(summary.verdicts_written, 6, "both backlogs in one pass");
        assert_eq!(summary.completed, 2);
        let order = verdict_contexts_in_order(&pool).await;
        assert_ne!(
            order[0], order[1],
            "the second anchor's first verdict comes before the first anchor's second: {order:?}"
        );
    }

    /// FAILS IF: the drain reads the content of a resource the erasure estate has fenced
    /// (`is_active` false), or leaves it out once it is active again.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_fenced_resources_artifact_is_not_backlog_until_it_is_active_again(pool: PgPool) {
        let w = seed_world(&pool, "fenced-reconcile").await;
        let artifacts = backlog_of(&pool, &w, 2).await;
        let fenced = artifacts[0];
        sqlx::query(
            "UPDATE kb_resources SET is_active = false \
              WHERE id = (SELECT resource_id FROM kb_data_artifacts WHERE id = $1)",
        )
        .bind(fenced.uuid())
        .execute(&pool)
        .await
        .unwrap();

        let summary = dispatch_tick(&pool, None).await.unwrap();

        assert_eq!(summary.verdicts_written, 1, "only the active resource's");
        assert_eq!(verdict_version(&pool, fenced).await, None);
        sqlx::query(
            "UPDATE kb_resources SET is_active = true \
              WHERE id = (SELECT resource_id FROM kb_data_artifacts WHERE id = $1)",
        )
        .bind(fenced.uuid())
        .execute(&pool)
        .await
        .unwrap();
        let anchor = HomeAnchor::Context(ContextId::from(w.context));
        let backlog: Vec<Uuid> = backlog_page(&pool, anchor, Uuid::nil(), 10)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.artifact_id)
            .collect();
        assert_eq!(
            backlog,
            vec![fenced.uuid()],
            "active again, it is backlog again"
        );
    }

    /// Make measuring each of `poisoned`'s content fail with SQLSTATE `code`, in this test's own
    /// throwaway database, by replacing the measure the drain calls.
    async fn poison_measurement(pool: &PgPool, poisoned: &[Uuid], code: &str) {
        let list: Vec<String> = poisoned.iter().map(|id| format!("'{id}'")).collect();
        sqlx::query(&format!(
            "CREATE OR REPLACE FUNCTION data_artifact_content_returned_bytes(p_artifact_id uuid) \
             RETURNS bigint LANGUAGE plpgsql AS $$ \
             BEGIN IF p_artifact_id IN ({}) THEN \
               RAISE EXCEPTION 'injected measurement fault' USING ERRCODE = '{code}'; \
             END IF; \
             RETURN (SELECT octet_length(content::text) FROM kb_data_artifact_content \
                      WHERE artifact_id = p_artifact_id); END $$",
            list.join(",")
        ))
        .execute(pool)
        .await
        .unwrap();
    }

    /// FAILS IF: a measured artifact does not break a run of failures, so failures scattered among
    /// healthy artifacts trip the breaker and stop the pass before the healthy ones are verdicted.
    #[sqlx::test(migrations = "../../migrations")]
    async fn failures_scattered_among_healthy_artifacts_never_trip_the_breaker(pool: PgPool) {
        let w = seed_world(&pool, "scattered-reconcile").await;
        let n = 2 * MAX_CONSECUTIVE_FAILED_ARTIFACTS as usize + 2;
        let artifacts = backlog_of(&pool, &w, n).await;
        let mut ordered: Vec<Uuid> = artifacts.iter().map(|a| a.uuid()).collect();
        ordered.sort();
        let poisoned: Vec<Uuid> = ordered.iter().copied().step_by(2).collect();
        poison_measurement(&pool, &poisoned, "XX000").await;

        let summary = dispatch_tick(&pool, None).await.unwrap();

        assert_eq!(
            summary.failed_artifacts as usize,
            poisoned.len(),
            "every poisoned artifact failed, and the pass reached them all"
        );
        assert_eq!(summary.verdicts_written as usize, n - poisoned.len());
        assert_eq!(
            summary.failed, 1,
            "the job is left in flight for the failures"
        );
    }

    /// FAILS IF: content Postgres refuses to render (a program-limit error) is counted as a
    /// failure that may pass, so its anchor's job is retried until it dies, rather than recorded.
    #[sqlx::test(migrations = "../../migrations")]
    async fn content_postgres_cannot_render_is_recorded_unrenderable(pool: PgPool) {
        let w = seed_world(&pool, "unrenderable-reconcile").await;
        let artifacts = backlog_of(&pool, &w, 2).await;
        let mut ordered: Vec<Uuid> = artifacts.iter().map(|a| a.uuid()).collect();
        ordered.sort();
        poison_measurement(&pool, &ordered[..1], "54000").await;

        let summary = dispatch_tick(&pool, None).await.unwrap();

        assert_eq!(summary.unreadable_artifacts, 1);
        assert_eq!(summary.failed_artifacts, 0);
        assert_eq!(summary.verdicts_written, 1);
        assert_eq!(summary.completed, 1);
        assert_eq!(
            unreadable_reason(&pool, ordered[0]).await.as_deref(),
            Some("content_unrenderable")
        );
    }

    /// FAILS IF: a real program-limit refusal is not read as one, or another error is.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_program_limit_refusal_is_told_apart_from_other_errors(pool: PgPool) {
        let limit = sqlx::query("SELECT repeat('x', 1100000000)")
            .execute(&pool)
            .await
            .expect_err("a 1.1 GB string is past Postgres's limit");
        assert!(is_program_limit(&limit), "{limit}");
        let other = sqlx::query("SELECT 1 / 0")
            .execute(&pool)
            .await
            .expect_err("division by zero");
        assert!(!is_program_limit(&other), "{other}");
    }
}
