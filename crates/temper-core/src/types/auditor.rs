//! Wire types for the citation auditor's dispatch tick (Set 5, Task 13).
//!
//! Spec `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §6.1.
//!
//! **The whole reason this module exists rather than reusing [`crate::types::workflow_job::ClaimedJob`]
//! is the grain mismatch, and it is worth stating once, here.** `audit_drift_sweep` is
//! **finding**-grained — `RETURNS TABLE(cogmap_id uuid, finding_id uuid, uncovered int)`
//! (`migrations/20260724000130_audit_drift_sweep.sql:86-87`) — while `kb_workflow_jobs` enforces
//! single-flight on `(cogmap_id, persona, dispatch_type)`
//! (`migrations/20260705000001_workflow_jobs.sql:43-45`, *"the single-flight guarantee"*). Enqueuing
//! one job per swept row would therefore create the first job and have
//! `workflow_job_enqueue`'s `ON CONFLICT DO NOTHING` (`:59-62`) **silently discard the other N−1** —
//! no error, no log, N findings collapsed into 1. So the tick groups the sweep by cogmap and
//! enqueues ONE job whose payload carries the work list ([`AuditJobPayload`]), and the claimed job
//! hands that list back ([`ClaimedAuditJob::citations`]) for the session to iterate.
//!
//! **The queue's grain is the cogmap; the auditor's grain is the CITATION** — not the finding, as
//! this module assumed until `20260727000050`. `audit_drift_sweep` still selects findings and its
//! ordering is still the only prioritization the auditor has, but each swept finding is expanded
//! through `resource_auditable_citations` before it reaches the payload, so the session receives
//! only citations this principal has not already weighed.
//!
//! No `ts-rs` derives here, deliberately, unlike `ClaimedJob`. The only TypeScript consumer is the
//! auditor schedule in `packages/agent-workflows/steward/`, which is **workspace-isolated** and has
//! no generated ts-rs tree (only temper-ui and the `mention` agent do — see CLAUDE.md). Its typed
//! view of this contract comes through `openapi.json` → `clients/temper-ts`, which the `utoipa`
//! derives below feed. Adding `ts(export)` would emit files into temper-ui's tree that nothing
//! imports, restaling `ts-rs-drift` for no consumer.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One row of `audit_drift_sweep` — a single cogmap-homed finding with incomplete audit coverage.
///
/// `uncovered` is `citation_magnitude - audit_coverage`, the size of the remainder the auditor has
/// not yet weighed. It ranks the uncovered findings — most-cited/least-audited first — but it is no
/// longer the whole ordering: since `20260726000010` the sweep interleaves the uncovered and stale
/// classes by rank, because a stale finding is fully covered by construction and a plain
/// `uncovered DESC` would park it behind every stuck finding forever (spec §6.3).
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSweepRow {
    /// The cogmap the finding is homed in — the queue's grain (spec §6.2).
    pub cogmap_id: Uuid,
    /// The finding with uncovered citations — the auditor's grain.
    pub finding_id: Uuid,
    /// Distinct live cited sources this finding carries that no audit has yet weighed.
    pub uncovered: i32,
}

/// One unit of the auditor's work: a `(block, source)` citation, and the finding it belongs to.
///
/// The finding rides along because it is not derivable from the pair at the point of use — the
/// session addresses its write as `POST /api/resources/{finding}/citation-audits`, and the
/// authorization subject is resolved server-side from `block_id` (`audit_gate.rs:65-77`), which
/// then refuses if the two disagree. Carrying it is what lets the session address the finding it
/// was actually given rather than one it inferred.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditCitation {
    /// The finding the citing block belongs to — the path segment of the audit write.
    pub finding_id: Uuid,
    /// The citing block.
    pub block_id: Uuid,
    /// The cited source. Always resource-kind; only resource citations are auditable.
    pub source_id: Uuid,
}

/// The payload written into `kb_workflow_jobs.payload` for a citation-audit job.
///
/// A typed struct, not an inline `serde_json::json!()` — the repo rule, and the reason it matters
/// here is that this shape crosses the DB and comes back out at claim time, so a silent key rename
/// would produce an empty list rather than a compile error.
///
/// **Citation-grained since `20260727000050`.** It carried `findings: Vec<Uuid>` until then, which
/// left the session unable to tell which citations it had already weighed — so it re-weighed all of
/// them, every tick. See that migration and task
/// `019fa5eb-2819-7e71-bd27-0d2875f60960`.
///
/// `#[serde(default)]` is deliberate and is the ONLY concession to the shape change: a job enqueued
/// before this deploy deserializes to an empty citation list rather than failing the claim. That
/// session then audits nothing and completes, and the sweep re-offers its findings on the next tick
/// — self-healing, and strictly better than a claim that errors on a payload nobody can re-write.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditJobPayload {
    /// Every citation in this cogmap that this principal should weigh, in the sweep's own finding
    /// order, with one finding's citations contiguous. The session iterates this list.
    #[serde(default)]
    pub citations: Vec<AuditCitation>,
}

/// A citation-audit job claimed for fan-out — the auditor twin of
/// [`crate::types::workflow_job::ClaimedJob`], carrying the citation list the job was enqueued with.
///
/// One isolated session per entry, exactly as the steward's fan-out works; the difference is that
/// each session iterates `citations` rather than tending one target.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimedAuditJob {
    /// The queue row id.
    pub id: Uuid,
    /// The single cognitive map this claimed run audits within.
    pub cogmap_id: Uuid,
    /// How many times this job has now been claimed (1 on first dispatch).
    pub attempts: i32,
    /// The citations this run must weigh — the payload the enqueue carried, read back verbatim.
    ///
    /// Every entry is work this principal has NOT already done: the enqueue resolved them through
    /// `resource_auditable_citations`, so the session needs no skip logic and is given no
    /// opportunity to exercise any. That is the whole point of the grain change — the invariant is
    /// held by what the session receives, not by an instruction it is asked to follow.
    pub citations: Vec<AuditCitation>,
}

/// Request body for `POST /api/auditor/dispatch`. Optional — the server default applies
/// ([`crate::types::workflow_job::DEFAULT_AUDITOR_DISPATCH_CAP`]); a supplied value is clamped into
/// `[1, MAX_AUDITOR_DISPATCH_CAP]` ([`crate::types::workflow_job::clamp_auditor_cap`]) and a value
/// below 1 is refused at the surface with 400.
///
/// There is no `threshold` twin of the steward's request: the auditor's selection predicate is
/// structural (`coverage < magnitude`, spec §6.3), not a tunable count, so there is nothing for a
/// caller to set.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditorDispatchTickRequest {
    /// Max **findings** the sweep returns this tick (`audit_drift_sweep`'s `p_limit`). Those
    /// findings collapse into at most that many jobs, one per distinct cogmap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<i64>,
}

/// Acknowledgement of an auditor session completing its dispatch job.
///
/// `job_id` is `None` when nothing OF THE CALLER'S was in flight for the cogmap — a manual audit
/// outside the dispatch loop, an already-completed job, or a lease the reaper expired. That is an
/// outcome, not an error: the session's verdicts stand either way, and the coverage sweep is what
/// decides whether the finding comes back. A caller that never claimed the job also lands here,
/// which is deliberate: completion is restricted to the claimant's own in-flight job (spec §6.5),
/// so a non-claimant's call is a guaranteed no-op rather than a refusal that would turn "there is
/// no job" into an error for the legitimate session.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditorJobCompleteAck {
    pub cogmap_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Uuid>,
}

/// Response for an auditor dispatch tick — the jobs claimed for fan-out.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditorDispatchTickResponse {
    pub claimed: Vec<ClaimedAuditJob>,
    /// The correlation the server parsed out of `x-auditor-correlation-id` and stamped onto every
    /// claimed job — echoed so the cron can assert its tick id survived parsing, rather than assume
    /// it. `None` when the header was absent or not a UUID (both self-root; neither is an error).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<Uuid>,
}
