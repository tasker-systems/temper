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
