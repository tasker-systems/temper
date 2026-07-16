//! Types for the persona-agnostic agent-dispatch job queue (`kb_workflow_jobs`, goal 019f3220).
//!
//! The queue serializes fan-out steward runs: at most one active job per
//! (cogmap, persona, dispatch_type). `Persona` and `DispatchType` are bounded sets we own — Rust
//! enums (serialized to `text`), so a new variant is a code change, never a migration.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

/// Which agent persona a queued job is for. The queue is persona-agnostic; `Embed` is the
/// non-agent, server-computed embedding worker (issue #299) and shares the queue with `Steward`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persona {
    Steward,
    Embed,
}

impl Persona {
    /// The wire/column string for this persona (stored in `kb_workflow_jobs.persona`).
    pub fn as_str(self) -> &'static str {
        match self {
            Persona::Steward => "steward",
            Persona::Embed => "embed",
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
}

impl DispatchType {
    /// The wire/column string for this dispatch type (stored in `kb_workflow_jobs.dispatch_type`).
    pub fn as_str(self) -> &'static str {
        match self {
            DispatchType::Steward => "steward",
            DispatchType::Embed => "embed",
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
