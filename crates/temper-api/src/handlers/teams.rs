//! Team lifecycle handlers — thin: extract `AuthUser`, dispatch one
//! `team_service` call, return the typed row. Service-direct (no Backend-trait
//! command, no event emission) per org-provisioning spec §2.6.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::middleware::auth::AuthUser;
use crate::services::team_service;
use crate::state::AppState;
use temper_core::types::ids::ProfileId;
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamMemberRow, TeamRow};

#[utoipa::path(
    get,
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
