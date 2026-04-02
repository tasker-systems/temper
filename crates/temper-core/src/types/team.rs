use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Team role — strict hierarchy: Owner > Maintainer > Member > Watcher.
///
/// Maps directly to the `team_role` Postgres enum. Four roles is small enough
/// that explicit matching in SQL functions and Rust logic is clearer than a
/// join-table permission model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
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
#[derive(Debug, Clone, FromRow)]
pub struct TeamMember {
    pub id: Uuid,
    pub team_id: Uuid,
    pub profile_id: Uuid,
    pub role: TeamRole,
    pub joined_at: DateTime<Utc>,
    pub invited_by_profile_id: Option<Uuid>,
}
