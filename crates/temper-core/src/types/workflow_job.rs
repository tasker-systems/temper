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
