use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::backend::DbBackend;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

use temper_core::types::ids::{ProfileId, ResourceId};
use temper_workflow::operations::{Backend, Surface, UpdateResource};
use temper_workflow::types::managed_meta::{MetaUpdatePayload, ResourceMetaResponse};
use temper_workflow::types::resource::ResourceRow;

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
    crate::backend::substrate_read::get_meta_select(
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
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<MetaUpdatePayload>,
) -> ApiResult<Json<ResourceRow>> {
    let cmd = UpdateResource {
        resource: ResourceId::from(resource_id),
        body: None,
        managed_meta: Some(payload.managed_meta),
        open_meta: Some(payload.open_meta),
        move_to: None,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend.update_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}
