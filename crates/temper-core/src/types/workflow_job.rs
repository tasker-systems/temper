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

/// Which agent persona a queued job is for. One variant today; the queue is persona-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persona {
    Steward,
}

impl Persona {
    /// The wire/column string for this persona (stored in `kb_workflow_jobs.persona`).
    pub fn as_str(self) -> &'static str {
        match self {
            Persona::Steward => "steward",
        }
    }
}

/// The kind of dispatch a job represents. Only `Steward` is queued today (materialize fans out
/// lease-free); the column is forward-looking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchType {
    Steward,
}

impl DispatchType {
    /// The wire/column string for this dispatch type (stored in `kb_workflow_jobs.dispatch_type`).
    pub fn as_str(self) -> &'static str {
        match self {
            DispatchType::Steward => "steward",
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
