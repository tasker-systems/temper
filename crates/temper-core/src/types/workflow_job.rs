//! Types for the persona-agnostic agent-dispatch job queue (`kb_workflow_jobs`, goal 019f3220).
//!
//! The queue serializes fan-out steward runs: at most one active job per
//! (cogmap, persona, dispatch_type). `Persona` and `DispatchType` are bounded sets we own — Rust
//! enums (serialized to `text`), so a new variant is a code change, never a migration.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::home::HomeAnchor;

/// Lease duration for a claimed job. MUST exceed the Vercel function timeout (300s default) so a
/// genuinely-running steward session never looks dead to the reaper.
pub const DEFAULT_STEWARD_LEASE_SECONDS: i32 = 600;

/// Default number of drifted maps dispatched per tick — the minimal budget guard. The sweep orders
/// most-drifted-first, so the cap is meaningful; richer prioritization is deferred.
pub const DEFAULT_STEWARD_DISPATCH_CAP: i64 = 10;

/// Lease for a claimed embed job (issue #299). Like the steward lease it MUST exceed the Vercel
/// function timeout (300s) so a genuinely-running embed is never reaped mid-flight.
pub const DEFAULT_EMBED_LEASE_SECONDS: i32 = 600;

/// Resources embedded per embed-dispatch tick. Conservative — each job embeds every deferred chunk of
/// a resource (ONNX inference), so a small cap per (frequent) cron tick keeps any one invocation well
/// under the function timeout; the queue drains the backlog across ticks.
pub const DEFAULT_EMBED_DISPATCH_CAP: i32 = 5;

/// Lease for a claimed citation-audit job (Set 5). Same reasoning as the steward lease: it MUST
/// exceed the Vercel function timeout (300s) so a genuinely-running auditor session is never reaped
/// mid-flight. An auditor session weighs several citations serially, so it is not shorter.
pub const DEFAULT_AUDITOR_LEASE_SECONDS: i32 = 600;

/// Rows the auditor's sweep asks for per tick — the `p_limit` handed to `audit_drift_sweep`, and
/// therefore a **finding** budget, not a cogmap one. The sweep is finding-grained
/// (`migrations/20260724000130_audit_drift_sweep.sql:86-87`,
/// `RETURNS TABLE(cogmap_id uuid, finding_id uuid, uncovered int)`) while the queue is cogmap-grained
/// (spec §6.1), so N swept findings collapse into ≤ N jobs. Deliberately larger than
/// [`DEFAULT_STEWARD_DISPATCH_CAP`] for that reason: a cap of 10 findings could be one cogmap's worth
/// of work and would starve every other map.
pub const DEFAULT_AUDITOR_DISPATCH_CAP: i64 = 50;

/// The ceiling on a caller-supplied auditor `cap`. Bounds the work ONE tick may ask the database
/// for: `audit_drift_sweep` scores every candidate finding (two producers, each a multi-table join)
/// before `p_limit` applies, and the same number is the claim's batch limit.
///
/// Generous rather than tight — a real backlog after an outage should drain in a few ticks, not
/// hundreds — but finite, which the shipped code was not.
pub const MAX_AUDITOR_DISPATCH_CAP: i64 = 500;

/// Resolve a caller-supplied auditor `cap` into the `int` the SQL takes.
///
/// **Not cosmetic.** The shipped spelling was `cap.unwrap_or(DEFAULT) as i32`, and an `i64 → i32`
/// `as` cast **wraps**: `{"cap": 2147483648}` became `-2147483648`, reaching Postgres as
/// `LIMIT -2147483648` — a database error rendered as a 500 from ordinary user input. `{"cap": -1}`
/// did the same directly. Clamping into `[1, MAX]` makes both impossible *and* bounds the sweep's
/// cost, and it lives in one place so every caller (the tick's claim limit and the sweep's finding
/// budget, which are deliberately the same number) cannot clamp differently.
///
/// The surfaces additionally REFUSE a non-positive `cap` with a 400 rather than silently accepting
/// it — the same two-layer shape as the audit value's range guard (a Rust `BadRequest` for the
/// caller's benefit, a hard bound underneath for everyone else's).
pub fn clamp_auditor_cap(cap: Option<i64>) -> i32 {
    cap.unwrap_or(DEFAULT_AUDITOR_DISPATCH_CAP)
        .clamp(1, MAX_AUDITOR_DISPATCH_CAP) as i32
}

/// Which agent persona a queued job is for. The queue is persona-agnostic; `Embed` is the
/// non-agent, server-computed embedding worker (issue #299) and shares the queue with `Steward`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persona {
    Steward,
    Embed,
    /// The citation auditor (Set 5). A DISTINCT persona value, not a steward dispatch_type: the
    /// single-flight index is `(cogmap_id, persona, dispatch_type)`
    /// (`migrations/20260705000001_workflow_jobs.sql:43-45`), so a separate persona is what lets an
    /// auditor job and a steward job be in flight over the same cogmap at once.
    Auditor,
    /// Region materialization moved off the request path (goal 019fc46c). Like `Embed`, a
    /// non-agent server-side worker sharing the queue. A DISTINCT persona for the same reason
    /// `Auditor` is one: single-flight keys on `(scope, persona, dispatch_type)`, and a region job
    /// must be able to be in flight over a cogmap that already has a steward or auditor job.
    Region,
}

impl Persona {
    /// The wire/column string for this persona (stored in `kb_workflow_jobs.persona`).
    pub fn as_str(self) -> &'static str {
        match self {
            Persona::Steward => "steward",
            Persona::Embed => "embed",
            Persona::Auditor => "auditor",
            Persona::Region => "region",
        }
    }
}

/// The kind of dispatch a job represents. `Steward` fans out an agent session per cogmap; `Embed`
/// backfills a resource's deferred chunk embeddings off the request path (issue #299). A new variant
/// is a code change, never a migration — the column is `text` and the queue keys are strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchType {
    Steward,
    Embed,
    /// One citation-audit pass over a single cogmap (Set 5). ONE dispatch type for the whole
    /// persona — deliberately not one per finding: per-finding discrimination would make
    /// `dispatch_type` unboundedly cardinal, which spec §6.1 names and rejects.
    CitationAudit,
    /// One region-clock tick over a single anchor (context or cogmap). ONE dispatch type for the
    /// whole persona, and deliberately covering BOTH clocks rather than one each: the two clocks
    /// share a watermark read and are cheap to run together, and splitting them would let a
    /// salience job and a formation job contend for the same anchor row — reintroducing, between
    /// two jobs, the serialization this move exists to remove.
    Materialize,
}

impl DispatchType {
    /// The wire/column string for this dispatch type (stored in `kb_workflow_jobs.dispatch_type`).
    pub fn as_str(self) -> &'static str {
        match self {
            DispatchType::Steward => "steward",
            DispatchType::Embed => "embed",
            DispatchType::CitationAudit => "citation-audit",
            DispatchType::Materialize => "materialize",
        }
    }
}

/// A job claimed for dispatch — the caller starts exactly one agent session per `ClaimedJob`,
/// carrying its single `cogmap_id` (the fan-out is over the workflow, never the agent's target).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedJob {
    /// The queue row id.
    pub id: Uuid,
    /// The single cognitive map this claimed run tends.
    pub cogmap_id: Uuid,
    /// How many times this job has now been claimed (1 on first dispatch).
    pub attempts: i32,
}

/// Outcome of one embed-dispatch invocation (issue #299): across the invocation's claim loop, how
/// many resource-keyed embed jobs were claimed, how many completed cleanly, how many failed (left for
/// the reaper's retry→dead path), and the total chunks embedded. Returned by the
/// `/api/embed/dispatch` drain so a cron/operator has observability. (An invocation loop-drains: it
/// keeps claiming until the queue is empty or the wall-clock deadline is hit, so these counts span
/// many claims, not one — see the `claimed` field.)
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmbedDispatchSummary {
    /// Dead embed jobs re-enqueued this invocation (Phase 4 re-drive). Zero unless the caller asked
    /// for a re-drive (`?redrive=true`); these resources are then eligible for the invocation's claim
    /// loop.
    pub redriven: u32,
    /// Total jobs claimed across the invocation's claim loop. Each loop iteration claims ≤ the
    /// dispatch cap, so the invocation total routinely **exceeds** `cap` — and a re-enqueued partial
    /// re-claimed on a later iteration is counted again. `claimed = 30` at `cap = 5` is a healthy long
    /// invocation, not a bug.
    pub claimed: u32,
    /// Jobs whose resource embedded cleanly and were marked done.
    pub completed: u32,
    /// Jobs whose embed errored — left in_progress for the reaper to retry (then dead at max attempts).
    pub failed: u32,
    /// Jobs that embedded their per-claim chunk budget but still hold stale chunks, and were
    /// **re-enqueued** to resume later (a subsequent claim this same invocation, or a later tick). Not
    /// a failure — the normal path for a resource larger than one claim's budget (prod's biggest holds
    /// 939 chunks against a budget of 64). A persistently non-zero `partial` with `chunks_embedded`
    /// climbing is a drain making progress, not a stuck one.
    pub partial: u32,
    /// Total chunks embedded across all jobs this invocation, whether they completed or were
    /// re-enqueued.
    pub chunks_embedded: u64,
}

/// Derived embedding-readiness of a resource (issue #299, Phase 4). Computed — never a stored column —
/// from the resource's current chunks plus its embed-job state (design §8), surfaced on the MCP
/// `EnrichedResource` so a caller can tell whether semantic (vector) search will find a just-created
/// resource yet. FTS is always immediate; only the vector is eventually-consistent under async embed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingStatus {
    /// Every current chunk is embedded (or the resource has no chunks at all — an empty body is
    /// trivially ready). Vector search will find it.
    Ready,
    /// ≥1 current chunk still has a NULL embedding and a live embed job exists
    /// (`pending`/`in_progress`/`waiting_for_retry`) — the vector is in flight.
    Pending,
    /// ≥1 current chunk still has a NULL embedding and no live job remains — the embed job is `dead`
    /// (reaper-exhausted) or absent after a supersede race. Recoverable via re-drive.
    Failed,
}

/// A resource-keyed job claimed for dispatch — the resource twin of [`ClaimedJob`]. The `Embed`
/// worker claims one of these per resource whose deferred chunk embeddings need backfilling (issue
/// #299); the `resource_id` is the scope it embeds.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedEmbedJob {
    /// The queue row id.
    pub id: Uuid,
    /// The resource whose deferred embeddings this claimed run backfills.
    pub resource_id: Uuid,
    /// How many times this job has now been claimed (1 on first dispatch).
    pub attempts: i32,
}

/// The payload a region-clock job carries: who occasioned the settling.
///
/// Once the tick leaves the request path the write no longer emits the `region_materialized` /
/// `salience_refreshed` events itself, so the emitter has to travel with the job or attribution is
/// lost — a system-emitted settling would satisfy the ledger's shape while telling a reader nothing
/// about which act occasioned it.
///
/// **The coalescing consequence, stated rather than discovered.** Single-flight keys on the tuple
/// alone, so when N arrivals collapse into one job the payload of the FIRST one wins and the later
/// emitters are dropped — the same "stale payload swallows this one silently" behaviour
/// `enqueue_with_payload` already documents. That is accepted here: the settling genuinely was
/// occasioned by many acts, the ledger records one of them, and the events that drove it remain
/// individually attributed in `kb_events`. It is a narrowing of *which act gets named*, never a loss
/// of the trail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionJobPayload {
    /// The entity whose write occasioned this settling — `kb_entities.id`, the same emitter the
    /// inline tick used to pass straight through to the clocks.
    pub emitter: Uuid,
}

/// An anchor-keyed job claimed for dispatch — the anchor twin of [`ClaimedJob`] and
/// [`ClaimedEmbedJob`]. The `Region` worker claims one of these per anchor whose region clocks are
/// due (goal 019fc46c).
///
/// The scope is a typed [`HomeAnchor`], not the raw `(cogmap_id, context_id)` pair the SQL returns:
/// the pair's "exactly one is non-null" invariant is enforced by `ck_workflow_jobs_one_scope` in the
/// database, and re-deriving it at every use site is how that invariant drifts. Parsing it once, at
/// the boundary, is what makes the worker unable to hold an ambiguous anchor at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimedAnchorJob {
    /// The queue row id.
    pub id: Uuid,
    /// The anchor whose region clocks this claimed run ticks.
    pub anchor: HomeAnchor,
    /// The entity to attribute this settling's events to — see [`RegionJobPayload`].
    pub emitter: Uuid,
    /// How many times this job has now been claimed (1 on first dispatch).
    pub attempts: i32,
}

#[cfg(test)]
mod cap_tests {
    use super::*;

    /// The wrap. `2147483648 as i32` is `-2147483648`, and a negative `LIMIT` is a Postgres error —
    /// so before the clamp this input produced a 500 from a plain JSON body. Asserting the CEILING
    /// (not merely "positive") is what makes this falsify a `max(1, …)`-only fix.
    #[test]
    fn a_cap_past_i32_is_clamped_to_the_ceiling_not_wrapped() {
        assert_eq!(clamp_auditor_cap(Some(2_147_483_648)), 500);
        assert_eq!(clamp_auditor_cap(Some(i64::MAX)), 500);
    }

    /// The other 500: `LIMIT -1`.
    #[test]
    fn a_non_positive_cap_is_clamped_to_one() {
        assert_eq!(clamp_auditor_cap(Some(-1)), 1);
        assert_eq!(clamp_auditor_cap(Some(0)), 1);
        assert_eq!(clamp_auditor_cap(Some(i64::MIN)), 1);
    }

    /// An ordinary cap and the omitted case are untouched — the clamp must not quietly change the
    /// tick's shipped budget.
    #[test]
    fn an_in_range_cap_and_the_default_pass_through() {
        assert_eq!(clamp_auditor_cap(Some(7)), 7);
        assert_eq!(clamp_auditor_cap(Some(500)), 500);
        assert_eq!(clamp_auditor_cap(None), DEFAULT_AUDITOR_DISPATCH_CAP as i32);
    }
}
