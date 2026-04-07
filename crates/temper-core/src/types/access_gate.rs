//! Types for the system access gate: join requests, system settings, and entitlements.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a join request in its lifecycle.
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
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
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

/// Instance-wide system settings (singleton row).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
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
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
pub struct Entitlements {
    pub system_access: bool,
    pub is_admin: bool,
    pub join_request_status: Option<JoinRequestStatus>,
}
