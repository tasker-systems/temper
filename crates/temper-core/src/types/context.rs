//! Context types — API request/response types for context CRUD.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::ids::ContextId;
use crate::context_ref::ContextOwnerRef;

/// Response row for context endpoints.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContextRow {
    pub id: ContextId,
    pub name: String,
    pub kb_owner_table: String,
    pub kb_owner_id: Uuid,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    /// The context's per-owner-unique slug (the natural-key half of `@owner/slug`).
    pub slug: String,
    /// The already-sigil'd owner addressable: `@<handle>` for profiles, `+<team-slug>` for teams.
    /// Together with `slug`, forms the full decorated context ref `{owner_ref}/{slug}`.
    pub owner_ref: String,
}

/// Context with resource count — used by the list endpoint.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContextRowWithCounts {
    pub id: ContextId,
    pub name: String,
    pub kb_owner_table: String,
    pub kb_owner_id: Uuid,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub resource_count: i64,
    /// The context's per-owner-unique slug (the natural-key half of `@owner/slug`).
    pub slug: String,
    /// The already-sigil'd owner addressable: `@<handle>` for profiles, `+<team-slug>` for teams.
    /// Together with `slug`, forms the full decorated context ref `{owner_ref}/{slug}`.
    pub owner_ref: String,
}

/// Request body for POST /api/contexts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContextCreateRequest {
    pub name: String,
    /// Who owns the new context. `None` (the default) creates a context owned by
    /// the calling profile, preserving pre-Chunk-3 behavior. `Team(slug)` creates
    /// a team-owned context (role-gated server-side to `owner`/`maintainer`).
    #[serde(default)]
    pub owner: Option<ContextOwnerRef>,
}

/// Request body for `POST /api/contexts/{id}/teams` — share a context into a team's read-reach.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareContextRequest {
    /// The team whose members (and DAG descendants) gain read-reach into the context.
    pub team_id: Uuid,
}

/// Result of sharing a context into a team. `shared` is `false` when the share already
/// existed (idempotent no-op).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareContextOutcome {
    pub context_id: Uuid,
    pub team_id: Uuid,
    /// `true` when this call inserted the share; `false` when it already existed.
    pub shared: bool,
}

/// Result of unsharing a context from a team. `unshared` is `false` when no share existed.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnshareContextOutcome {
    pub context_id: Uuid,
    pub team_id: Uuid,
    /// `true` when this call deleted a share; `false` when none existed.
    pub unshared: bool,
}

/// Request body for `POST /api/contexts/{id}/reassign` — transfer a context's ownership to
/// a team. Binding a context to a team is the single path to shared authorship (read-only
/// sharing stays `share_context`; writing into a context requires team ownership).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReassignContextRequest {
    /// The team that will own the context. Members with an authoring role can then write
    /// into it via the container-write cascade.
    pub to_team_id: Uuid,
}

/// Result of a context ownership transfer. `reassigned` is `false` when the context was
/// already owned by the target team (idempotent no-op).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReassignContextOutcome {
    pub context_id: Uuid,
    /// The new `+team-slug` decorated owner ref.
    pub owner_ref: String,
    /// `true` when this call transferred ownership; `false` when it was already team-owned.
    pub reassigned: bool,
}
