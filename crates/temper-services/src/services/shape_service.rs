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
use temper_substrate::shape_schema;
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
/// sentence naming the bound; one within it that cannot be compiled (malformed, past the
/// parsed-size budget, not a field tree, or with a pattern past
/// `shape_schema::MAX_PATTERN_BYTES`) is refused with the no-fetch constructor's sentence, naming
/// what it refused first and where — the declarer wrote the schema (shape ruling R4). Both are a
/// `DataArtifactRefusal`, so a schema no ask could use is refused when it is declared rather than
/// stored. Before either, a
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
        // The declarer wrote the schema, so the refusal names what the compile refused first.
        SchemaVerdict::Uncompilable(refused) => {
            return Err(ApiError::DataArtifactRefusal(refused.detailed()))
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
    /// The shape does not compile ([`shape_schema::compile`]): its stored schema breaks a rule the
    /// compile holds, such as the field-tree vocabulary, the scalar rule for `const` and `enum`
    /// members, or the parsed-size budget. Declare compiles every schema before it stores one, so
    /// such a row was stored before the rule it breaks.
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
mod tests;
