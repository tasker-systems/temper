use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::error::ApiResult;
use temper_services::services::context_service::{
    self, ContextCreateRequest, ContextRow, ContextRowWithCounts, ShareContextOutcome,
    ShareContextRequest, UnshareContextOutcome,
};
use temper_services::state::AppState;

#[utoipa::path(
    get,
    operation_id = "list_contexts",
    path = "/api/contexts",
    tag = "Contexts",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of visible contexts with resource counts", body = Vec<ContextRowWithCounts>),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<ContextRowWithCounts>>> {
    context_service::list_visible(&state.pool, ProfileId::from(auth.0.profile.id))
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    operation_id = "create_context",
    path = "/api/contexts",
    tag = "Contexts",
    security(("bearer_auth" = [])),
    request_body = ContextCreateRequest,
    responses(
        (status = 201, description = "Context created", body = ContextRow),
        (status = 409, description = "Context name already exists"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ContextCreateRequest>,
) -> ApiResult<(StatusCode, Json<ContextRow>)> {
    let caller = ProfileId::from(auth.0.profile.id);
    let (owner_table, owner_id) =
        context_service::resolve_create_owner(&state.pool, caller, body.owner.as_ref()).await?;
    let row = context_service::create(&state.pool, &owner_table, owner_id, &body.name).await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[utoipa::path(
    get,
    operation_id = "get_context",
    path = "/api/contexts/{id}",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Context details", body = ContextRow),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
) -> ApiResult<Json<ContextRow>> {
    context_service::get_visible(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        ContextId::from(context_id),
    )
    .await
    .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/contexts/{id}/teams",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    request_body = ShareContextRequest,
    responses(
        (status = 200, description = "Context shared (or idempotent no-op)", body = ShareContextOutcome),
        (status = 403, description = "Caller is not a system admin"),
    )
)]
pub async fn share_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Json(body): Json<ShareContextRequest>,
) -> ApiResult<Json<ShareContextOutcome>> {
    let outcome = context_service::share(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        context_id,
        &body,
    )
    .await?;
    Ok(Json(outcome))
}

#[utoipa::path(
    delete,
    path = "/api/contexts/{id}/teams/{team_id}",
    tag = "Contexts",
    params(
        ("id" = Uuid, Path, description = "Context ID"),
        ("team_id" = Uuid, Path, description = "Team ID to unshare"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Context unshared (or no-op)", body = UnshareContextOutcome),
        (status = 403, description = "Caller is not a system admin"),
    )
)]
pub async fn unshare_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((context_id, team_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<UnshareContextOutcome>> {
    let outcome = context_service::unshare(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        context_id,
        team_id,
    )
    .await?;
    Ok(Json(outcome))
}
