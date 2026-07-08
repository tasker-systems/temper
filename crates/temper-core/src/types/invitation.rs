use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::team::TeamRole;

/// Invitation status — lifecycle of a team invitation.
///
/// Maps directly to the `invitation_status` Postgres enum.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invitation.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, serde::Serialize, serde::Deserialize)]
#[sqlx(type_name = "invitation_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
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
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invitation.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, FromRow, serde::Serialize, serde::Deserialize)]
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

/// A pending invitation resolved to the *invitee's* view — the `TeamInvitation`
/// fields plus the team's slug/name for display. Returned by
/// `GET /api/invitations/mine`; the caller is authorized to redeem these, so the
/// `token` is legitimately theirs to see.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invitation.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, FromRow, serde::Serialize, serde::Deserialize)]
pub struct InviteeInvitation {
    pub id: Uuid,
    pub team_id: Uuid,
    pub team_slug: String,
    pub team_name: String,
    pub invited_email: String,
    pub invited_by_profile_id: Uuid,
    pub role: TeamRole,
    pub token: String,
    pub status: InvitationStatus,
    pub expires_at: DateTime<Utc>,
    pub created: DateTime<Utc>,
}

/// Request body for `POST /api/teams/{id}/invite`.
///
/// `role` cannot be `Owner` — the service rejects it (ownership is transferred,
/// not invited).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invitation.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateInvitationRequest {
    pub invited_email: String,
    pub role: TeamRole,
}

/// Response from `POST /api/invitations/{token}/accept` — the team the caller
/// just joined and at what role.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invitation.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AcceptInvitationResponse {
    pub team_id: Uuid,
    pub team_slug: String,
    pub role: TeamRole,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invitee_invitation_serde_roundtrip() {
        let json = serde_json::json!({
            "id": "019f41f3-74ab-7ec0-8b0d-cb21662c51cb",
            "team_id": "019f25d6-e1a9-7360-8a35-6bdf8ef53940",
            "team_slug": "platform",
            "team_name": "Platform",
            "invited_email": "person@x.com",
            "invited_by_profile_id": "019d4add-f49d-7c43-a87d-dda470e5dd9c",
            "role": "member",
            "token": "abc123",
            "status": "pending",
            "expires_at": "2026-07-15T00:00:00Z",
            "created": "2026-07-08T00:00:00Z"
        });
        let inv: InviteeInvitation = serde_json::from_value(json).unwrap();
        assert_eq!(inv.team_slug, "platform");
        assert_eq!(inv.role, TeamRole::Member);
        assert_eq!(inv.status, InvitationStatus::Pending);
    }
}
