use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::middleware::auth::AuthUser;
use crate::services::context_service::{
    self, ContextCreateRequest, ContextRow, ContextRowWithCounts,
};
use crate::state::AppState;
use temper_core::types::ids::{ContextId, ProfileId};

#[utoipa::path(
    get,
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
    let row = context_service::create(&state.pool, ProfileId::from(auth.0.profile.id), &body.name)
        .await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[utoipa::path(
    get,
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
