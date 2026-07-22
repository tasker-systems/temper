use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Profile — the temper-domain identity.
///
/// Bridges external auth identity to everything temper cares about:
/// team membership, resource ownership, preferences, vault configuration.
/// A profile is "who I am in temper" regardless of which provider I
/// authenticated through. No auth provider fields — those live in
/// `ProfileAuthLink`.
///
/// Auto-provisioned on first authenticated request. Deactivation is a
/// principal-standing state (`kb_principal_standing.state = 'deactivated'`),
/// not a column on this row — the legacy `is_active` flag was dropped in
/// principal-admission Phase 2.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "profile.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct Profile {
    pub id: Uuid,
    pub display_name: String,
    pub slug: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub preferences: serde_json::Value,
    pub vault_config: serde_json::Value,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// Links an external auth provider identity to a temper profile.
///
/// A profile can have multiple auth links (e.g., Google and GitHub with the
/// same email). Identity reconciliation: when a new provider identity arrives
/// with an email matching an existing link, it auto-links to the same profile.
/// One link is marked `is_default` as the primary identity.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "profile.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ProfileAuthLink {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub auth_provider: String,
    pub auth_provider_user_id: String,
    pub email: Option<String>,
    /// Whether the identity provider has verified `email` (persisted from the
    /// `email_verified` claim at provisioning; refreshed on verified sign-ins).
    /// Email-based matching (reconciliation, invitation resolution) requires it.
    pub email_verified: bool,
    pub is_default: bool,
    pub linked_at: DateTime<Utc>,
}

/// Result of validating whether a profile can be deactivated.
///
/// Deactivation is blocked if the profile is the sole owner of any active team
/// (must transfer ownership first) or owns resources with no other access path
/// (must transfer or share first).
#[derive(Debug, Clone)]
pub enum DeactivationCheck {
    /// Safe to deactivate
    Ready,
    /// Must resolve these issues first
    Blocked {
        /// Teams where this profile is the only owner
        sole_owner_teams: Vec<Uuid>,
        /// Count of resources owned by this profile not in any team
        orphaned_resource_count: u32,
    },
}
