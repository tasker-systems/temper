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
/// **The flow is not link-based, and never has been.** `invited_email` is a
/// *correlator*, matched at sign-in — nothing mails a token-bearing URL, and no
/// UI route redeems one. The invitee authenticates, reads their own pending
/// invitations from `GET /api/invitations/mine` (which returns `token`, since it
/// is legitimately theirs), and redeems it through `POST /api/invitations/accept`
/// with the token in the **request body**. CLI: `temper team invite`,
/// `temper team join`, `temper team request-join`.
///
/// This comment previously described the link-based flow as the primary one. It
/// was aspirational, it was load-bearing in the wrong direction — it is what made
/// a reviewer believe retiring the old token-in-path route had to account for
/// invitation URLs sitting in inboxes — and there are none.
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

/// Request body for `POST /api/invitations/accept` and
/// `POST /api/invitations/decline`.
///
/// ## Why the token is a body field and not a path segment
///
/// The token is a **bearer capability** — `invitation_service` mints 128 CSPRNG
/// bits and the authority to join the team *is* the token, for seven days. A URL
/// path is the least private part of a request: intermediaries log it as a matter
/// of course, it rides in `Referer` headers, it lands in browser history, and it
/// is recorded as a span attribute that leaves the building on export. A request
/// body goes to none of those places.
///
/// ## One type for both routes
///
/// Accept and decline differ in what they *do*, and that difference is carried by
/// the route and the verb. What the body means is identical in both — "which
/// invitation, named by the capability token that authorizes acting on it" — so
/// this is one intent written once, not two shapes that happen to match.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invitation.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InvitationTokenRequest {
    pub token: String,
}

/// Response from `POST /api/invitations/accept` — the team the caller
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
