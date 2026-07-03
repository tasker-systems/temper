//! Resource ownership reassignment handlers — thin: extract `AuthUser`, dispatch one
//! `reassign_service` call. Service-direct, same precedent as `invitations`.

use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::ids::ProfileId;
use temper_core::types::reassign::{
    BulkReassignAck, BulkReassignRequest, ReassignAck, ReassignResourceRequest,
};
use temper_services::error::ApiResult;
use temper_services::services::reassign_service;
use temper_services::state::AppState;

#[utoipa::path(
    post,
    path = "/api/resources/{id}/reassign",
    tag = "Reassign",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    request_body = ReassignResourceRequest,
    responses(
        (status = 200, description = "Owner reassigned", body = ReassignAck),
        (status = 403, description = "Forbidden (not owner, or admin reach not satisfied)"),
        (status = 404, description = "Resource has no home / not found"),
    )
)]
pub async fn reassign_resource(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(body): Json<ReassignResourceRequest>,
) -> ApiResult<Json<ReassignAck>> {
    reassign_service::reassign_resource(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        resource_id,
        body.to_profile_id,
    )
    .await?;
    Ok(Json(ReassignAck {
        resource_id,
        to_profile_id: body.to_profile_id,
    }))
}

#[utoipa::path(
    post,
    path = "/api/teams/{id}/reassign",
    tag = "Reassign",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    request_body = BulkReassignRequest,
    responses(
        (status = 200, description = "Team resources reassigned", body = BulkReassignAck),
        (status = 403, description = "Forbidden (caller does not manage the team, or target not a member)"),
    )
)]
pub async fn reassign_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<BulkReassignRequest>,
) -> ApiResult<Json<BulkReassignAck>> {
    let ids = reassign_service::reassign_team_resources(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        body.from_profile_id,
        body.to_profile_id,
    )
    .await?;
    Ok(Json(BulkReassignAck { resource_ids: ids }))
}
