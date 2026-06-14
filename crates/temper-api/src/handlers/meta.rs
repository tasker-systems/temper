use axum::extract::{Path, State};
use axum::Extension;
use axum::Json;
use uuid::Uuid;

use crate::backend::select_backend;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::auth::{AuthUser, DeviceId};
use crate::services::meta_service;
use crate::state::AppState;

use temper_core::operations::{ResourceRef, Surface, UpdateResource};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::managed_meta::{MetaUpdatePayload, ResourceMetaResponse};
use temper_core::types::resource::ResourceRow;

#[utoipa::path(
    get,
    path = "/api/resources/{id}/meta",
    tag = "Meta",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Current managed/open meta for the resource", body = ResourceMetaResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn get_meta(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<ResourceMetaResponse>> {
    meta_service::get_meta(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        ResourceId::from(resource_id),
    )
    .await
    .map(Json)
}

#[utoipa::path(
    put,
    path = "/api/resources/{id}/meta",
    tag = "Meta",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    request_body = MetaUpdatePayload,
    responses(
        (status = 200, description = "Updated resource", body = ResourceRow),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Forbidden", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn update_meta(
    State(state): State<AppState>,
    auth: AuthUser,
    device_id: Option<Extension<DeviceId>>,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<MetaUpdatePayload>,
) -> ApiResult<Json<ResourceRow>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());

    let cmd = UpdateResource {
        resource: ResourceRef::Uuid {
            id: ResourceId::from(resource_id),
        },
        body: None,
        managed_meta: Some(payload.managed_meta),
        open_meta: Some(payload.open_meta),
        move_to: None,
        origin: Surface::ApiHttp,
    };
    let backend = select_backend(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        device_id,
        Surface::ApiHttp,
    )
    .map_err(ApiError::from)?;
    let out = backend.update_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}
