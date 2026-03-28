use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Serialize;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::middleware::auth::AuthUser;
use crate::services::resource_service::{
    self, ResourceCreateRequest, ResourceListParams, ResourceRow, ResourceUpdateRequest,
};
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ResourceListParams>,
) -> ApiResult<Json<Vec<ResourceRow>>> {
    resource_service::list_visible(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<ResourceRow>> {
    resource_service::get_visible(&state.pool, auth.0.profile.id, resource_id)
        .await
        .map(Json)
}

#[derive(Debug, Serialize)]
pub struct ContentResponse {
    pub resource_id: Uuid,
    pub markdown: String,
}

pub async fn get_content(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<ContentResponse>> {
    let markdown =
        resource_service::get_content(&state.pool, auth.0.profile.id, resource_id).await?;
    Ok(Json(ContentResponse {
        resource_id,
        markdown,
    }))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ResourceCreateRequest>,
) -> ApiResult<Json<ResourceRow>> {
    resource_service::create(&state.pool, auth.0.profile.id, req)
        .await
        .map(Json)
}

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

#[derive(Debug, Serialize)]
pub struct DeleteResponse {
    pub deleted: bool,
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<DeleteResponse>> {
    resource_service::delete(&state.pool, auth.0.profile.id, resource_id).await?;
    Ok(Json(DeleteResponse { deleted: true }))
}
