//! Resource ownership reassignment handlers — thin: extract `AuthUser`, dispatch one
//! `reassign_service` call. Service-direct, same precedent as `invitations`.

use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::reassign::{
    BulkReassignAck, BulkReassignRequest, ReassignAck, ReassignResourceRequest,
};
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::reassign_service;
use temper_services::state::AppState;

/// Reassign a resource to another owner
#[utoipa::path(
    post,
    path = "/api/resources/{id}/reassign",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    request_body = ReassignResourceRequest,
    responses(
        (status = 200, description = "Owner reassigned", body = ReassignAck),
        (status = 400, description = "The caller owns the resource, but it is homed in a cognitive map (map interiors are not reassignable)", body = ErrorBody),
        (status = 403, description = "Forbidden: not the owner and no admin reach over the resource and target. An unknown id answers the same 403"),
        (status = 404, description = "Not answered: an unknown id answers 403, as a resource the caller has no authority over"),
        (status = 410, description = "The resource was erased (code RESOURCE_ERASED); answered to a caller with authority who also holds the erased resource (its owner, or an admin with reach who holds a read grant on it); everyone else gets 403", body = ErrorBody),
    )
)]
pub async fn reassign_resource(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(body): Json<ReassignResourceRequest>,
) -> ApiResult<Json<ReassignAck>> {
    reassign_service::reassign_resource(&state.pool, &auth.0, resource_id, body.to_profile_id)
        .await?;
    Ok(Json(ReassignAck {
        resource_id,
        to_profile_id: body.to_profile_id,
    }))
}

/// Bulk reassign a team's resources
#[utoipa::path(
    post,
    path = "/api/teams/{id}/reassign",
    tag = "Teams",
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
        &auth.0,
        team_id,
        body.from_profile_id,
        body.to_profile_id,
    )
    .await?;
    Ok(Json(BulkReassignAck { resource_ids: ids }))
}
