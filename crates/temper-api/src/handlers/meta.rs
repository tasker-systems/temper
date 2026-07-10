use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;

use temper_core::types::ids::{ProfileId, ResourceId};
use temper_workflow::operations::{Backend, UpdateResource};
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
    temper_services::backend::substrate_read::get_meta_select(
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
    RequestSurface(surface): RequestSurface,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<MetaUpdatePayload>,
) -> ApiResult<Json<ResourceRow>> {
    let act = payload.act.into_act_context().map_err(ApiError::from)?;
    let cmd = UpdateResource {
        resource: ResourceId::from(resource_id),
        // Meta-only path is Property-only (Fork 2): identity changes go through
        // the full update path, never the /meta endpoint.
        title: None,
        slug: None,
        body: None,
        managed_meta: Some(payload.managed_meta),
        open_meta: Some(payload.open_meta),
        // Meta-only path is Property-only (Fork 2); the goal edge (relationship-fated) is not a
        // property and travels via the full update path, never the /meta endpoint.
        goal: None,
        move_to: None,
        context_ref: None,
        act,
        origin: surface,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend.update_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}
