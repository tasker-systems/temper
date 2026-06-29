use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Team role — strict hierarchy: Owner > Maintainer > Member > Watcher.
///
/// Maps directly to the `team_role` Postgres enum. Four roles is small enough
/// that explicit matching in SQL functions and Rust logic is clearer than a
/// join-table permission model.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "team_role", rename_all = "snake_case")]
pub enum TeamRole {
    Owner,
    Maintainer,
    Member,
    Watcher,
}

/// A team — the unit of collaboration in temper.
///
/// Teams are fully owned by temper, not delegated to the auth provider.
/// This means the team model survives auth provider swaps. A team must
/// always have exactly one owner. Soft-deleted via `is_active = false`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[derive(Debug, Clone, FromRow)]
pub struct Team {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub metadata: serde_json::Value,
    pub created_by_profile_id: Uuid,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// A profile's membership in a team with a specific role.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[derive(Debug, Clone, FromRow)]
pub struct TeamMember {
    pub id: Uuid,
    pub team_id: Uuid,
    pub profile_id: Uuid,
    pub role: TeamRole,
    pub joined_at: DateTime<Utc>,
    pub invited_by_profile_id: Option<Uuid>,
}

// ---------------------------------------------------------------------------
// Wire types — these match the REAL substrate columns (`kb_teams`,
// `kb_team_members`). The legacy `Team`/`TeamMember` structs above predate the
// WS6 collapse and describe columns that no longer exist; they are unused (dead)
// and must NOT be used for queries. Use the `*Row`/`*Request` types below.
// ---------------------------------------------------------------------------

/// Response row for team endpoints — matches the real `kb_teams` columns.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TeamRow {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub created: DateTime<Utc>,
    /// NULL = not an auto-join ("everyone") team; otherwise the role new
    /// members are enrolled at. Admin-gated to set.
    pub auto_join_role: Option<TeamRole>,
}

/// Request body for `POST /api/teams`.
///
/// `parent` is an optional team ref (`+slug` or bare `slug`); when present the
/// new team is created as a child in `kb_teams_parents` and the caller must be
/// `owner`/`maintainer` on the parent. `auto_join_role` is admin-gated.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamCreateRequest {
    pub slug: String,
    pub name: Option<String>,
    pub parent: Option<String>,
    pub auto_join_role: Option<TeamRole>,
}

/// Response row for team membership — matches the real `kb_team_members` columns.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TeamMemberRow {
    pub team_id: Uuid,
    pub profile_id: Uuid,
    pub role: TeamRole,
    pub created: DateTime<Utc>,
}

/// Request body for `POST /api/teams/{id}/members`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMemberRequest {
    pub profile_id: Uuid,
    pub role: TeamRole,
}
