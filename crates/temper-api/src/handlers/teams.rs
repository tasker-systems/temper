//! Team lifecycle handlers — thin: extract `AuthUser`, dispatch one
//! `team_service` call, return the typed row. Service-direct (no Backend-trait
//! command, no event emission) per org-provisioning spec §2.6.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::ids::ProfileId;
use temper_core::types::team::{
    AddMemberRequest, ChangeRoleRequest, TeamCreateRequest, TeamDetail, TeamMemberRow, TeamRow,
    TeamUpdateRequest,
};
use temper_services::error::ApiResult;
use temper_services::services::team_service;
use temper_services::state::AppState;

#[utoipa::path(
    get,
    operation_id = "list_teams",
    path = "/api/teams",
    tag = "Teams",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Teams the caller is a member of", body = Vec<TeamRow>),
    )
)]
pub async fn list(State(state): State<AppState>, auth: AuthUser) -> ApiResult<Json<Vec<TeamRow>>> {
    team_service::list_teams(&state.pool, ProfileId::from(auth.0.profile.id))
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    operation_id = "create_team",
    path = "/api/teams",
    tag = "Teams",
    security(("bearer_auth" = [])),
    request_body = TeamCreateRequest,
    responses(
        (status = 201, description = "Team created", body = TeamRow),
        (status = 403, description = "Forbidden (child requires owner/maintainer; auto_join_role requires admin)"),
        (status = 409, description = "Team slug already exists"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<TeamCreateRequest>,
) -> ApiResult<(StatusCode, Json<TeamRow>)> {
    let row =
        team_service::create_team(&state.pool, ProfileId::from(auth.0.profile.id), &body).await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[utoipa::path(
    post,
    path = "/api/teams/{id}/members",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    request_body = AddMemberRequest,
    responses(
        (status = 201, description = "Member added", body = TeamMemberRow),
        (status = 403, description = "Forbidden (caller is not owner/maintainer)"),
        (status = 400, description = "Cannot grant owner via add_member; use ownership transfer"),
    )
)]
pub async fn add_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<AddMemberRequest>,
) -> ApiResult<(StatusCode, Json<TeamMemberRow>)> {
    let row = team_service::add_member(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        &body,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[utoipa::path(
    get,
    path = "/api/teams/{id}",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Team detail + members", body = TeamDetail),
        (status = 404, description = "Team not found or not visible to caller"),
    )
)]
pub async fn detail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
) -> ApiResult<Json<TeamDetail>> {
    team_service::team_detail(&state.pool, ProfileId::from(auth.0.profile.id), team_id)
        .await
        .map(Json)
}

#[utoipa::path(
    patch,
    operation_id = "update_team",
    path = "/api/teams/{id}",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    request_body = TeamUpdateRequest,
    responses(
        (status = 200, description = "Team metadata updated", body = TeamRow),
        (status = 403, description = "Forbidden (caller is not owner/maintainer)"),
        (status = 404, description = "Team not found or soft-deleted"),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<TeamUpdateRequest>,
) -> ApiResult<Json<TeamRow>> {
    team_service::update_team(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        &body,
    )
    .await
    .map(Json)
}

#[utoipa::path(
    delete,
    operation_id = "delete_team",
    path = "/api/teams/{id}",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Team soft-deleted"),
        (status = 403, description = "Forbidden (caller is not the owner)"),
        (status = 404, description = "Team not found or already soft-deleted"),
        (status = 409, description = "Cannot delete the temper-system root team"),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    team_service::delete_team(&state.pool, ProfileId::from(auth.0.profile.id), team_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/teams/{id}/members/{profile_id}",
    tag = "Teams",
    params(
        ("id" = Uuid, Path, description = "Team ID"),
        ("profile_id" = Uuid, Path, description = "Member profile ID"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Member removed"),
        (status = 403, description = "Forbidden (not owner/maintainer and not self)"),
        (status = 404, description = "Member not found"),
        (status = 409, description = "Cannot remove last owner or SAML-provisioned row"),
    )
)]
pub async fn remove_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, profile_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    team_service::remove_member(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        profile_id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    patch,
    path = "/api/teams/{id}/members/{profile_id}",
    tag = "Teams",
    params(
        ("id" = Uuid, Path, description = "Team ID"),
        ("profile_id" = Uuid, Path, description = "Member profile ID"),
    ),
    security(("bearer_auth" = [])),
    request_body = ChangeRoleRequest,
    responses(
        (status = 200, description = "Role changed", body = TeamMemberRow),
        (status = 400, description = "Cannot grant owner via role change"),
        (status = 403, description = "Forbidden (not owner/maintainer)"),
        (status = 404, description = "Member not found"),
        (status = 409, description = "Cannot demote last owner or SAML-provisioned row"),
    )
)]
pub async fn change_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, profile_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<ChangeRoleRequest>,
) -> ApiResult<Json<TeamMemberRow>> {
    team_service::change_role(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        profile_id,
        body.role,
    )
    .await
    .map(Json)
}
