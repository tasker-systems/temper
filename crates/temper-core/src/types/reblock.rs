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
//! `temper-mcp` (tool params), and `temper-client` (typed request builder).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The candidate-window bound applied when an invocation omits one. Conservative on purpose —
/// it is a convenience, never a correctness bound: the receipt's continuation cursor, not the
/// number, is what keeps a walk resumable and complete. A later measurement of the
/// public-function envelope may redeclare it.
pub const DEFAULT_REBLOCK_LIMIT: i64 = 100;

/// What one invocation covers. On the wire the one-target arms are single-key objects and the
/// deployment-wide arm is the bare string, so exactly one target is structural — there is no
/// shape that means "whichever" — and `all` must be spelled out to be meant.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReblockScope {
    /// Exactly one resource, by id.
    Resource(Uuid),
    /// Every live, complete candidate homed in one context, enumerated through the caller's own
    /// visibility.
    Context(Uuid),
    /// Deployment-wide. Claims a reach ordinary visibility does not yield, so it is
    /// SystemAdmin-gated and must be asked for by name — never the default.
    All,
}

/// What happened to one candidate. The act's arms are `Reblocked`, `NoOp`, `Denied`, `InProgress`,
/// `Byteless`, `Drift`, and `Error`; `WouldChange` exists only on the survey arm (`dry_run`) — the
/// act's counterpart there is `Reblocked`. `Denied` is the gate's refusal; the other three op
/// refusals are the op's own, each carrying the human `detail` that names the candidate and its
/// remediation (declined counts in the summary cover all four).
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReblockOutcome {
    /// The act fired `resource_reblocked` — the ledger carries the act under the batch
    /// correlation id.
    Reblocked { event: Uuid },
    /// The partition already matched; the act fired nothing (the ledger is indistinguishable
    /// from never having run — a receipt fact, not a ledger one).
    NoOp,
    /// Survey only: the partition would move — the act, run now, would re-block.
    WouldChange,
    /// The per-resource gate train refused the invoking operator — the grant boundary doing its
    /// work, visible per row and auditable. The batch never rolls back over it. Deliberately
    /// carries no detail: the refusal is opaque by design.
    Denied,
    /// The candidate's body is still arriving (`in_progress`): a partition decision over a
    /// still-arriving body would be a guess. Retry the candidate once its ingest completes.
    InProgress {
        /// What happened and what to do about it.
        detail: String,
    },
    /// The candidate has no stored verbatim bytes to compose a body from — no live blocks, or a
    /// block in a derived shape whose bytes were never stored.
    Byteless {
        /// What happened and what to do about it.
        detail: String,
    },
    /// A fresh chunking of the candidate's body does not reproduce its stored chunking, so the
    /// stored partition cannot serve as the re-block's baseline.
    Drift {
        /// What happened and what to do about it.
        detail: String,
    },
    /// The act errored; the batch declined-and-continued. The row names what happened so the
    /// operator can retry it alone.
    Error { message: String },
}

/// One candidate's receipt row.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReblockCandidate {
    /// The candidate the row is about.
    pub resource: Uuid,
    pub outcome: ReblockOutcome,
}

/// Per-class counts over the invocation's candidates.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReblockSummary {
    pub reblocked: u64,
    pub would_change: u64,
    pub no_op: u64,
    pub declined: u64,
    pub error: u64,
    /// Candidates homed in the scope that are still arriving (`in_progress`) — not considered
    /// by this invocation. They are partitioned when their upload finalizes; address one
    /// directly (resource scope) to act on it now.
    pub in_progress: u64,
}

/// The invocation response — the receipt. No durable receipt table exists: the ledger holds
/// every real act, this holds the batch's truth, and the batch correlation id pairs them.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReblockReceipt {
    /// Whether this receipt came from the survey arm (`dry_run`) — outcomes classify without
    /// touching anything.
    pub dry_run: bool,
    /// The batch correlation id stamped on every ledger act this invocation fired. A grouping
    /// key, never a capability.
    pub correlation_id: Uuid,
    /// One row per candidate considered.
    pub outcomes: Vec<ReblockCandidate>,
    pub summary: ReblockSummary,
    /// The last candidate id considered — pass it back as the next invocation's `after_id` to
    /// resume. `None` when nothing was considered. Stateless: it rides the receipt, and a
    /// re-run from the top is always safe anyway.
    pub after_id: Option<Uuid>,
}

/// Request body for `POST /api/resources/reblock` — one bounded, resumable re-blocking step.
///
/// The `scope` names what this invocation covers; exactly one target, with the deployment-wide
/// arm named explicitly. `dry_run` selects the read-only survey; `limit` bounds the candidate
/// window (the conservative default applies when omitted); `after_id` resumes a walk from the
/// previous receipt's cursor. The response is the receipt: per-candidate outcomes, per-class
/// counts, the batch correlation id, and the continuation cursor.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReblockRequest {
    /// What this invocation covers.
    pub scope: ReblockScope,
    /// Survey instead of act: classify every candidate without touching anything. Survey first,
    /// then run with `false`, then survey again to verify.
    pub dry_run: bool,
    /// The candidate-window bound; the conservative default applies when omitted. A convenience,
    /// never a correctness bound — the cursor keeps the walk resumable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    /// Resume key from the previous receipt — only candidates after it are considered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_id: Option<Uuid>,
}
