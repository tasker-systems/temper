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

/// A team the context is shared to (read-reach) at transfer time — reach the new owner inherits.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InheritedShare {
    pub team_id: Uuid,
    /// Decorated `+team-slug` ref.
    pub team_ref: String,
}

/// An explicit context read-grant that survives the ownership flip — inherited residual reach.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InheritedReadGrant {
    /// `kb_profiles` or `kb_teams`.
    pub principal_table: String,
    pub principal_id: Uuid,
    /// Decorated ref: `@handle` for a profile, `+slug` for a team.
    pub principal_ref: String,
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
    /// Read-reach the new owner inherits: teams this context was shared to (kb_team_contexts).
    /// Surfaced, not swept — the new owner prunes deliberately.
    #[serde(default)]
    pub inherited_shares: Vec<InheritedShare>,
    /// Read-reach the new owner inherits: explicit context read-grants (kb_access_grants).
    #[serde(default)]
    pub inherited_read_grants: Vec<InheritedReadGrant>,
}

/// Request body for `POST /api/contexts/{id}/rename` — change a context's display name.
/// The addressable slug is **derived** from the name server-side; there is deliberately no
/// independent slug parameter. Rename is one field.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameContextRequest {
    /// The new display name. Canonicalized (trimmed, internal whitespace collapsed) before it
    /// is stored, and sluggified to derive the new addressable slug.
    pub name: String,
}

/// Result of a context rename. `renamed` is `false` when the canonical name already equalled
/// the stored one (idempotent no-op).
///
/// Carries the **composed** `context_ref` as well as the `owner_ref` + `slug` halves it is built
/// from: the caller has just had their address changed out from under them, so making them
/// reconstruct the new one from parts is the wrong place to save a field.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameContextOutcome {
    pub context_id: Uuid,
    /// The stored (canonical) name after the call — on a no-op, the name it already had.
    pub name: String,
    /// The per-owner-unique slug after the call, derived from `name`.
    pub slug: String,
    /// The already-sigil'd owner addressable, unchanged by a rename: `@<handle>` for profiles,
    /// `+<team-slug>` for teams.
    pub owner_ref: String,
    /// The full decorated context ref after the call — `{owner_ref}/{slug}`, the address the
    /// caller should use from now on.
    ///
    /// Composed **once**, server-side, by the service that produces this outcome; no surface
    /// reassembles it. Note the trap: `owner_ref` is already sigil-decorated, whereas
    /// `context_ref::decorated_context_ref`'s `owner_addressable` parameter is the *bare*
    /// handle/team-slug. The two spellings must never be mixed, or the ref reads `@@handle/slug`.
    pub context_ref: String,
    /// `true` when this call changed the stored name; `false` when the canonical name was
    /// already the stored one (no event emitted, nothing written).
    pub renamed: bool,
}
