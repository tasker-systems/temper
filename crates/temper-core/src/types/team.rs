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
    /// Optional human description; NULL until set via `PATCH /api/teams/{id}`.
    pub description: Option<String>,
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

/// Provenance of a team membership row. Maps to the `team_member_source`
/// Postgres enum (added by `20260702000001_saml_group_provisioning.sql`).
/// `Idp` rows are owned by SAML reconcile and are not user-mutable.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "team_member_source", rename_all = "snake_case")]
pub enum TeamMemberSource {
    Native,
    Idp,
}

/// A team member enriched with the profile handle and provenance — the row
/// shape returned inside `TeamDetail`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TeamMemberDetail {
    pub profile_id: Uuid,
    pub handle: String,
    pub role: TeamRole,
    pub source: TeamMemberSource,
}

/// Full team detail — the team row plus its member roster. Response body for
/// `GET /api/teams/{id}`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamDetail {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub created: DateTime<Utc>,
    pub auto_join_role: Option<TeamRole>,
    pub members: Vec<TeamMemberDetail>,
}

/// Request body for `PATCH /api/teams/{id}` — update team metadata.
///
/// Both fields are optional: a `None` leaves that column unchanged (partial
/// merge via SQL COALESCE). Sending neither is a no-op that returns the row.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamUpdateRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Request body for `PATCH /api/teams/{id}/members/{profile_id}`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeRoleRequest {
    pub role: TeamRole,
}
