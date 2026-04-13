use axum::extract::{Path, State};
use axum::Extension;
use axum::Json;
use serde_json::Value;
use uuid::Uuid;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::{AuthUser, DeviceId};
use crate::services::meta_service;
use crate::state::AppState;

use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::managed_meta::{MetaUpdatePayload, ResourceMetaResponse};

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
        (status = 200, description = "Meta updated", body = Value),
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
) -> ApiResult<Json<Value>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());
    meta_service::update_meta(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        ResourceId::from(resource_id),
        &device_id,
        payload,
    )
    .await
    .map(Json)
}
