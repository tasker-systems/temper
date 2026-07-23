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

// The `AccessMode` enum was retired with the `access_mode` control (spec §14 / D18): standing now
// answers per-principal what a global mode switch answered instance-wide, so no code branches on the
// mode any more. Phase 2 finishes the retirement — the `access_mode` wire field is gone from both
// settings structs below, and the `kb_system_settings.access_mode` column drops in Phase 2's
// operator-run migration. Re-introducing a typed mode here would be the first step of re-coupling
// admission to a global switch — which is exactly what standing replaced.

/// Instance-wide system settings (singleton row).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SystemSettings {
    pub id: i32,
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
    pub terms_version: Option<String>,
    pub terms_resource_uri: Option<String>,
    pub instance_name: Option<String>,
}

impl From<SystemSettings> for PublicSystemSettings {
    fn from(s: SystemSettings) -> Self {
        Self {
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
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAccessDetails {
    pub email: Option<String>,
    pub display_name: Option<String>,
    /// Why this principal was refused, typed (spec §7). The sole refusal signal on the 403 since
    /// Phase 2 retired the legacy `join_request_status` field new clients never branched on. The
    /// typed refusal distinguishes "never granted" (`denied`) from "granted and revoked" (`revoked`)
    /// — a distinction that matters to the user and in an audit.
    ///
    /// Carried as `temper_principal::Refusal` so every surface branches on it exhaustively — the
    /// Rust ones through the enum, the generated temper-ts / temper-rb clients through the
    /// discriminated `kind` union that crate's feature-gated derives now emit.
    pub refusal: temper_principal::Refusal,
    pub request_url: Option<String>,
    pub cli_command: Option<String>,
}
