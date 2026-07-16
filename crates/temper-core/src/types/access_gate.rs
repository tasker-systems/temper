//! Types for the system access gate: join requests, system settings, and entitlements.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a join request in its lifecycle.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "join_request_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum JoinRequestStatus {
    Pending,
    Approved,
    Rejected,
    Withdrawn,
}

/// A user-initiated request to join a team (typically the gating team).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct JoinRequest {
    pub id: Uuid,
    pub team_id: Uuid,
    pub requesting_profile_id: Uuid,
    pub status: JoinRequestStatus,
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
    pub accepted_terms_at: Option<DateTime<Utc>>,
    pub reviewed_by_profile_id: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// A join request with the requesting profile's display info (for admin queue).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct JoinRequestWithProfile {
    pub id: Uuid,
    pub team_id: Uuid,
    pub requesting_profile_id: Uuid,
    pub status: JoinRequestStatus,
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
    pub accepted_terms_at: Option<DateTime<Utc>>,
    pub reviewed_by_profile_id: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    // Joined from kb_profiles
    pub display_name: String,
    pub email: Option<String>,
}

/// The system access gate mode — the `kb_system_settings.access_mode` set.
///
/// Stored as a `VARCHAR(16)` CHECK column (not a PG enum), so it is parsed at
/// the logic boundary via [`Self::from_db_str`] rather than decoded by sqlx.
/// Typing the gate decision removes the stringly `== "open"` branch and makes a
/// new mode a compile error at every match site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    Open,
    InviteOnly,
}

impl AccessMode {
    /// Canonical DB string form (the `access_mode` CHECK values).
    pub fn as_db_str(self) -> &'static str {
        match self {
            AccessMode::Open => "open",
            AccessMode::InviteOnly => "invite_only",
        }
    }

    /// Parse the `access_mode` column value. Returns `None` for any value
    /// outside the CHECK set (which the DB constraint should make impossible).
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "open" => Some(AccessMode::Open),
            "invite_only" => Some(AccessMode::InviteOnly),
            _ => None,
        }
    }
}

/// Instance-wide system settings (singleton row).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SystemSettings {
    pub id: i32,
    pub access_mode: String,
    pub gating_team_slug: Option<String>,
    pub terms_version: Option<String>,
    pub terms_resource_uri: Option<String>,
    pub instance_name: Option<String>,
    pub updated: DateTime<Utc>,
}

/// Public-facing system settings (no gating_team_slug — prevents info leakage).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicSystemSettings {
    pub access_mode: String,
    pub terms_version: Option<String>,
    pub terms_resource_uri: Option<String>,
    pub instance_name: Option<String>,
}

impl From<SystemSettings> for PublicSystemSettings {
    fn from(s: SystemSettings) -> Self {
        Self {
            access_mode: s.access_mode,
            terms_version: s.terms_version,
            terms_resource_uri: s.terms_resource_uri,
            instance_name: s.instance_name,
        }
    }
}

/// Entitlements included in the profile response — tells the client
/// what this profile is allowed to do at the system level.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct Entitlements {
    pub system_access: bool,
    pub is_admin: bool,
    pub join_request_status: Option<JoinRequestStatus>,
}

/// The command a rejected caller runs to request system access.
///
/// Lives here, beside the `SystemAccessDetails` it rides in, rather than as a
/// literal at the surface that builds the payload: temper-api cannot see the
/// clap tree, so a string authored there is gated by nothing. Here, temper-cli
/// depends on temper-core and pins it against the real parser.
pub const REQUEST_ACCESS_COMMAND: &str = "temper auth request-access --message \"...\"";

/// Details included in the SystemAccessRequired error response.
///
/// SECURITY NOTE: The `email` and `display_name` fields are safe to include
/// because the caller already proved ownership of this identity through OAuth.
/// We are reflecting the caller's own profile back — not disclosing another
/// user's information. Do not add fields that reveal other users' data.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAccessDetails {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub access_mode: String,
    pub join_request_status: Option<JoinRequestStatus>,
    pub request_url: Option<String>,
    pub cli_command: Option<String>,
}
