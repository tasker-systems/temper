use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::auth::AuthenticatedProfile;

/// Access level for a resource within a team scope.
///
/// Maps directly to the `access_level` Postgres enum.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "access.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "access_level", rename_all = "snake_case")]
pub enum AccessLevel {
    /// Collaborative ownership — any team member (member role or above) can
    /// modify or delete. Deletion means full removal from the temper system.
    /// Essential for shared tickets, milestones, research notes, session notes.
    Vault,

    /// Team members can read and edit content, but only the resource owner can
    /// remove it from the team or delete it entirely. Useful for shared specs,
    /// plans, reference documents.
    Mutable,

    /// Read-only for all team members. The owner controls all mutations,
    /// sharing decisions, and removal. Useful for published research,
    /// finalized decisions, reference material.
    Immutable,
}

/// A resource's scoped presence in a team with an explicit access level.
///
/// A resource can belong to multiple teams simultaneously with different
/// access levels per team.
#[derive(Debug, Clone, FromRow)]
pub struct TeamResource {
    pub id: Uuid,
    pub team_id: Uuid,
    pub resource_id: Uuid,
    pub access_level: AccessLevel,
    pub added_by_profile_id: Uuid,
    pub added_at: DateTime<Utc>,
}

/// Marker trait for types that participate in access-scoped queries.
///
/// The actual enforcement is in SQL via `resources_visible_to()`,
/// `can_modify_resource()`, and `can_manage_team()`. This trait provides
/// the Rust-side interface for constructing scoped query parameters.
/// The database is the authority; Rust is the caller.
pub trait AccessScoped {
    /// The profile ID to scope visibility to
    fn profile_id(&self) -> Uuid;
    /// Optional team scope narrowing (None = all teams the profile belongs to)
    fn team_id(&self) -> Option<Uuid>;
}

impl AccessScoped for AuthenticatedProfile {
    fn profile_id(&self) -> Uuid {
        self.profile.id
    }

    fn team_id(&self) -> Option<Uuid> {
        None // default: visible across all teams
    }
}
