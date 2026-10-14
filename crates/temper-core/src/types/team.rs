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
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "team_role", rename_all = "snake_case")]
pub enum TeamRole {
    Owner,
    Maintainer,
    Member,
    Watcher,
}

impl TeamRole {
    /// Numeric rank for the strict hierarchy Owner > Maintainer > Member > Watcher.
    ///
    /// `TeamRole` deliberately does not derive `Ord` — the derive would rank by declaration
    /// order, which puts `Owner` *lowest* and silently inverts every comparison. Callers that
    /// need to compare authority use this instead.
    ///
    /// A role added to this enum must be given a rank here, and the compiler's exhaustiveness
    /// check is what makes that non-optional. That is the point: policy ceilings expressed as
    /// `a.rank() <= b.rank()` (see `machine_authz`) then bar the new role by construction
    /// rather than by someone remembering to extend a hand-listed set.
    #[must_use]
    pub fn rank(self) -> u8 {
        match self {
            TeamRole::Owner => 3,
            TeamRole::Maintainer => 2,
            TeamRole::Member => 1,
            TeamRole::Watcher => 0,
        }
    }
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
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "team_member_source", rename_all = "snake_case")]
pub enum TeamMemberSource {
    Native,
    Idp,
}

/// The path a recorded team-membership change came through — the `via` of a
/// `team_membership_changed` admin event (team-role rank spec, "The event").
///
/// A closed set with a fixed SQL spelling: `_admin_team_membership_write` refuses any value not in
/// it, and [`MembershipVia::as_str`] is the one Rust spelling of that set. The derived writers
/// (personal-team genesis, auto-join materialization, the machine-cap trigger) have no `via`
/// because they are not recorded (spec C8, R13).
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipVia {
    /// An add, change or remove of another member, by a manager of the team or a system admin.
    MemberAct,
    /// A member acting on themselves: leaving, or lowering their own role.
    #[serde(rename = "self")]
    SelfAct,
    /// An accepted invitation (the invitee is the actor; the payload names the inviter).
    Invitation,
    /// `create_team`'s creator row.
    TeamGenesis,
    /// A system admin approving an access request (`access_service::review_request`).
    AccessApproval,
    /// A machine registration's team reach (`machine_registration_service::apply_reach`).
    MachineReach,
    /// SAML reconcile of identity-provider rows. The system is the actor.
    IdpReconcile,
}

impl MembershipVia {
    /// The wire and SQL spelling — what `_admin_team_membership_write`'s `p_via` accepts.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MembershipVia::MemberAct => "member_act",
            MembershipVia::SelfAct => "self",
            MembershipVia::Invitation => "invitation",
            MembershipVia::TeamGenesis => "team_genesis",
            MembershipVia::AccessApproval => "access_approval",
            MembershipVia::MachineReach => "machine_reach",
            MembershipVia::IdpReconcile => "idp_reconcile",
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `as_str` is the SQL spelling and serde is the payload spelling; the ledger reads one and
    /// `_admin_team_membership_write` writes the other, so they must be the same string.
    #[test]
    fn membership_via_as_str_matches_its_serde_spelling() {
        for via in [
            MembershipVia::MemberAct,
            MembershipVia::SelfAct,
            MembershipVia::Invitation,
            MembershipVia::TeamGenesis,
            MembershipVia::AccessApproval,
            MembershipVia::MachineReach,
            MembershipVia::IdpReconcile,
        ] {
            assert_eq!(
                serde_json::to_value(via).unwrap(),
                serde_json::Value::String(via.as_str().to_string())
            );
        }
    }
}
