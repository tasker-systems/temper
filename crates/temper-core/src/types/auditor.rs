//! Wire types for the citation auditor's dispatch tick (Set 5, Task 13).
//!
//! Spec `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §6.1.
//!
//! **The whole reason this module exists rather than reusing [`crate::types::workflow_job::ClaimedJob`]
//! is the grain mismatch, and it is worth stating once, here.** `audit_drift_sweep` is
//! **finding**-grained — `RETURNS TABLE(cogmap_id uuid, finding_id uuid, uncovered int)`
//! (`migrations/20260723000030_audit_drift_sweep.sql:86-87`) — while `kb_workflow_jobs` enforces
//! single-flight on `(cogmap_id, persona, dispatch_type)`
//! (`migrations/20260705000001_workflow_jobs.sql:43-45`, *"the single-flight guarantee"*). Enqueuing
//! one job per swept row would therefore create the first job and have
//! `workflow_job_enqueue`'s `ON CONFLICT DO NOTHING` (`:59-62`) **silently discard the other N−1** —
//! no error, no log, N findings collapsed into 1. So the tick groups the sweep by cogmap and
//! enqueues ONE job whose payload carries the finding list ([`AuditJobPayload`]), and the claimed
//! job hands that list back ([`ClaimedAuditJob::findings`]) for the session to iterate.
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
/// not yet weighed; the sweep orders by it descending, so the most-cited/least-audited findings head
/// the queue (spec §6.3).
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

/// The payload written into `kb_workflow_jobs.payload` for a citation-audit job.
///
/// A typed struct, not an inline `serde_json::json!()` — the repo rule, and the reason it matters
/// here is that this shape crosses the DB and comes back out at claim time, so a silent key rename
/// would produce an empty finding list rather than a compile error.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditJobPayload {
    /// Every finding in this cogmap the sweep found with uncovered citations, most-uncovered-first.
    /// The session iterates this list; it is never a single id.
    pub findings: Vec<Uuid>,
}

/// A citation-audit job claimed for fan-out — the auditor twin of
/// [`crate::types::workflow_job::ClaimedJob`], carrying the finding list the job was enqueued with.
///
/// One isolated session per entry, exactly as the steward's fan-out works; the difference is that
/// each session iterates `findings` rather than tending one target.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimedAuditJob {
    /// The queue row id.
    pub id: Uuid,
    /// The single cognitive map this claimed run audits within.
    pub cogmap_id: Uuid,
    /// How many times this job has now been claimed (1 on first dispatch).
    pub attempts: i32,
    /// The findings this run must audit — the payload the enqueue carried, read back verbatim.
    pub findings: Vec<Uuid>,
}

/// Request body for `POST /api/auditor/dispatch`. Optional — the server default applies
/// ([`crate::types::workflow_job::DEFAULT_AUDITOR_DISPATCH_CAP`]).
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
/// `job_id` is `None` when no job was active for the cogmap — a manual audit outside the dispatch
/// loop, or a job the reaper already expired. That is an outcome, not an error: the session's
/// verdicts stand either way, and the coverage sweep is what decides whether the finding comes back.
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
