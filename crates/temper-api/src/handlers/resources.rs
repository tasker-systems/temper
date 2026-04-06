use axum::extract::{Path, Query, State};
use axum::Extension;
use axum::Json;
use uuid::Uuid;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::{AuthUser, DeviceId};
use crate::services::resource_service::{
    self, ResourceCreateRequest, ResourceListParams, ResourceRow, ResourceUpdateRequest,
};
use crate::state::AppState;

use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::resource::{ContentResponse, DeleteResponse};

#[utoipa::path(
    get,
    path = "/api/resources",
    tag = "Resources",
    params(ResourceListParams),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of visible resources", body = Vec<ResourceRow>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ResourceListParams>,
) -> ApiResult<Json<Vec<ResourceRow>>> {
    resource_service::list_visible(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
}

#[utoipa::path(
    get,
    path = "/api/resources/{id}",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Resource metadata", body = ResourceRow),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<ResourceRow>> {
    resource_service::get_visible(&state.pool, auth.0.profile.id, resource_id)
        .await
        .map(Json)
}

#[utoipa::path(
    get,
    path = "/api/resources/{id}/content",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Reconstituted markdown content", body = ContentResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn get_content(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<ContentResponse>> {
    let markdown =
        resource_service::get_content(&state.pool, auth.0.profile.id, resource_id).await?;
    Ok(Json(ContentResponse {
        resource_id: ResourceId::from(resource_id),
        markdown,
    }))
}

#[utoipa::path(
    post,
    path = "/api/resources",
    tag = "Resources",
    request_body = ResourceCreateRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Created resource", body = ResourceRow),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 409, description = "Conflict", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ResourceCreateRequest>,
) -> ApiResult<Json<ResourceRow>> {
    resource_service::create(&state.pool, auth.0.profile.id, req)
        .await
        .map(Json)
}

#[utoipa::path(
    patch,
    path = "/api/resources/{id}",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    request_body = ResourceUpdateRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Updated resource", body = ResourceRow),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Forbidden", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(req): Json<ResourceUpdateRequest>,
) -> ApiResult<Json<ResourceRow>> {
    resource_service::update(&state.pool, auth.0.profile.id, resource_id, req)
        .await
        .map(Json)
}

#[utoipa::path(
    delete,
    path = "/api/resources/{id}",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Resource deleted", body = DeleteResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Forbidden", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    device_id: Option<Extension<DeviceId>>,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<DeleteResponse>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());
    resource_service::delete(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        ResourceId::from(resource_id),
        &device_id,
    )
    .await?;
    Ok(Json(DeleteResponse { deleted: true }))
}
