//! Team invitation handlers — thin: extract `AuthUser`, dispatch one
//! `invitation_service` call, return the typed row. Service-direct (no
//! Backend-trait command, no event emission), same precedent as `teams`.
//!
//! Route tiers matter: `invite` / `list` are team-admin actions and live in the
//! system-access-gated router; `accept` / `decline` live in the un-gated router
//! so an invitee to the gating team can redeem *before* they hold system access.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::ids::ProfileId;
use temper_core::types::invitation::{
    AcceptInvitationResponse, CreateInvitationRequest, InviteeInvitation, TeamInvitation,
};
use temper_services::error::ApiResult;
use temper_services::services::invitation_service;
use temper_services::state::AppState;

#[utoipa::path(
    post,
    path = "/api/teams/{id}/invite",
    tag = "Invitations",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    request_body = CreateInvitationRequest,
    responses(
        (status = 201, description = "Invitation created", body = TeamInvitation),
        (status = 400, description = "Owner role cannot be invited"),
        (status = 403, description = "Forbidden (caller is not owner/maintainer)"),
        (status = 409, description = "A pending invitation already exists for this email"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<CreateInvitationRequest>,
) -> ApiResult<(StatusCode, Json<TeamInvitation>)> {
    let params = invitation_service::CreateInvitationParams {
        invited_email: body.invited_email,
        role: body.role,
    };
    let inv = invitation_service::create_invitation(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        params,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(inv)))
}

#[utoipa::path(
    get,
    path = "/api/teams/{id}/invitations",
    tag = "Invitations",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Pending invitations for the team", body = Vec<TeamInvitation>),
        (status = 403, description = "Forbidden (caller is not owner/maintainer)"),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
) -> ApiResult<Json<Vec<TeamInvitation>>> {
    invitation_service::list_invitations(&state.pool, ProfileId::from(auth.0.profile.id), team_id)
        .await
        .map(Json)
}

#[utoipa::path(
    get,
    path = "/api/invitations/mine",
    tag = "Invitations",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The caller's own pending invitations", body = Vec<InviteeInvitation>),
    )
)]
pub async fn list_mine(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<InviteeInvitation>>> {
    invitation_service::list_for_profile(&state.pool, ProfileId::from(auth.0.profile.id))
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/invitations/{token}/accept",
    tag = "Invitations",
    params(("token" = String, Path, description = "Invitation token")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Invitation redeemed; caller joined the team", body = AcceptInvitationResponse),
        (status = 400, description = "Invitation expired or already declined"),
        (status = 404, description = "Unknown token"),
        (status = 409, description = "Invitation already redeemed by another profile"),
    )
)]
pub async fn accept(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(token): Path<String>,
) -> ApiResult<Json<AcceptInvitationResponse>> {
    invitation_service::accept_invitation(&state.pool, ProfileId::from(auth.0.profile.id), &token)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/invitations/{token}/decline",
    tag = "Invitations",
    params(("token" = String, Path, description = "Invitation token")),
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Invitation declined"),
        (status = 400, description = "Invitation was already accepted"),
        (status = 404, description = "Unknown token"),
    )
)]
pub async fn decline(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(token): Path<String>,
) -> ApiResult<StatusCode> {
    invitation_service::decline_invitation(&state.pool, ProfileId::from(auth.0.profile.id), &token)
        .await
        .map(|()| StatusCode::NO_CONTENT)
}
