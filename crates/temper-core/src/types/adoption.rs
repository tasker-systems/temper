//! Cross-surface types for corpus adoption — the operator's bounded, resumable, verifiable walk
//! bringing an existing corpus under the blocking policy.
//!
//! Adoption is N ordinary re-block writes, never a new write class: each per-resource act is
//! gated by the acting principal's existing write predicates, and the batch mints no authority.
//! Bounded by invocation, trigger-never-engine: every invocation declares a limit and returns a
//! continuation cursor, and nothing runs between invocations. The receipt is the invocation
//! response — per-resource outcomes under one batch correlation id — and the survey arm
//! (`dry_run`) is the same machinery read-only, the verifiability instrument (survey → act →
//! re-survey). Idempotence is per-resource and derived: the op's own no-op decision, re-computed
//! from the live partition on every pass — no watermark, no conformance marker.
//!
//! Mirrors [`crate::types::materialize`]: shared between `temper-api` (OpenAPI schema source),
//! `temper-mcp` (tool params), and `temper-client` (typed request builder) once the wire
//! surfaces land.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What one invocation covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdoptScope {
    /// Exactly one resource, by id.
    Resource(Uuid),
    /// Every live, complete candidate homed in one context, enumerated through the caller's own
    /// visibility.
    Context(Uuid),
    /// Deployment-wide. Claims a reach ordinary visibility does not yield, so it is
    /// SystemAdmin-gated and must be asked for by name — never the default.
    All,
}

/// Why one candidate produced no act.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdoptDeclined {
    /// The per-resource gate train refused the invoking operator — the grant boundary doing its
    /// work, visible per row and auditable. The batch never rolls back over it.
    Denied,
    /// The op's own refusal, rendered verbatim: a still-arriving (`in_progress`) body, a block
    /// with no stored verbatim bytes (a derived shape), or a stored chunking a fresh chunking
    /// of the body does not reproduce (chunker drift). Each names its remediation, and none is
    /// an adoption failure.
    Op { reason: String },
}

/// What happened to one candidate. `WouldChange` exists only on the survey arm (`dry_run`); the
/// act's counterpart is `Reblocked`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdoptOutcome {
    /// The act fired `resource_reblocked` — the ledger carries the act under the batch
    /// correlation id.
    Reblocked { event: Uuid },
    /// The partition already matched; the act fired nothing (the ledger is indistinguishable
    /// from never having run — a receipt fact, not a ledger one).
    NoOp,
    /// Survey only: the partition would move — the act, run now, would re-block.
    WouldChange,
    /// No act, for a typed reason.
    Declined(AdoptDeclined),
    /// The act errored; the batch declined-and-continued. The row names what happened so the
    /// operator can retry it alone.
    Error { message: String },
}

/// One candidate's receipt row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptResourceOutcome {
    /// The candidate the row is about.
    pub resource: Uuid,
    pub outcome: AdoptOutcome,
}

/// Per-class counts over the invocation's candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptSummary {
    pub reblocked: u64,
    pub would_change: u64,
    pub no_op: u64,
    pub declined: u64,
    pub error: u64,
}

/// The invocation response — the receipt. No durable receipt table exists: the ledger holds
/// every real act, this holds the batch's truth, and the batch correlation id pairs them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptReceipt {
    /// Whether this receipt came from the survey arm (`dry_run`) — outcomes classify without
    /// touching anything.
    pub dry_run: bool,
    /// The batch correlation id stamped on every ledger act this invocation fired. A grouping
    /// key, never a capability.
    pub correlation_id: Uuid,
    /// One row per candidate considered.
    pub outcomes: Vec<AdoptResourceOutcome>,
    pub summary: AdoptSummary,
    /// The last candidate id considered — pass it back as the next invocation's `after_id` to
    /// resume. `None` when nothing was considered. Stateless: it rides the receipt, and a
    /// re-run from the top is always safe anyway.
    pub after_id: Option<Uuid>,
}
