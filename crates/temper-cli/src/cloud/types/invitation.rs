use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::team::TeamRole;

/// Invitation status — lifecycle of a team invitation.
///
/// Maps directly to the `invitation_status` Postgres enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "invitation_status", rename_all = "snake_case")]
pub enum InvitationStatus {
    Pending,
    Accepted,
    Declined,
    Expired,
}

/// A pending or resolved invitation to join a team.
///
/// Primary flow is link-based: invite generates a token-bearing URL,
/// recipient clicks, authenticates, profile auto-created if needed,
/// joins team. CLI commands: `temper team invite`, `temper team join`,
/// `temper team request-join`.
///
/// Constraints:
/// - `role` cannot be `Owner` — ownership is only transferred, never invited
/// - One pending invite per email per team
/// - 7-day default expiry, checked at acceptance time
/// - Acceptance is idempotent
#[derive(Debug, Clone, FromRow)]
pub struct TeamInvitation {
    pub id: Uuid,
    pub team_id: Uuid,
    pub invited_email: String,
    pub invited_by_profile_id: Uuid,
    pub role: TeamRole,
    pub token: String,
    pub status: InvitationStatus,
    pub expires_at: DateTime<Utc>,
    pub created: DateTime<Utc>,
}
