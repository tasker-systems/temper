//! HTTP handlers for the segmented (multi-block) ingest surface: append one segment, finalize
//! the session, and read the currently-landed set back (the resume/progress query).
//!
//! Thin handlers only: `AuthUser` extractor → `DbBackend::new` → dispatch the `Backend` trait
//! method (Task 2.2) → map errors via `ApiError`. The auth-before-write gate
//! (`can_modify_resource`) lives in the `DbBackend` methods, not here — mirrors
//! `handlers::ingest`.
//!
//! Segmented **begin** (block 0) is not here — it is the existing `POST /api/ingest` create path
//! (`handlers::ingest::create`), branching on `IngestPayload.segmented`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult};
use temper_services::state::AppState;

use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::ingest::{AppendBlockPayload, BlocksResponse, FinalizePayload};
use temper_workflow::operations::Backend;

#[utoipa::path(
    post,
    operation_id = "append_block",
    path = "/api/resources/{id}/blocks",
    tag = "Ingest",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    request_body = AppendBlockPayload,
    responses(
        (status = 200, description = "Segment landed (or already landed — idempotent); currently-landed set returned", body = BlocksResponse),
        (status = 400, description = "Invalid chunks_packed"),
        (status = 403, description = "Caller cannot modify this resource"),
    )
)]
pub async fn append_block_handler(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<AppendBlockPayload>,
) -> ApiResult<Json<BlocksResponse>> {
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .append_block(ResourceId::from(resource_id), payload, surface)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}

#[utoipa::path(
    post,
    operation_id = "finalize_resource",
    path = "/api/resources/{id}/finalize",
    tag = "Ingest",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    request_body = FinalizePayload,
    responses(
        (status = 204, description = "Segmented ingest finalized"),
        (status = 400, description = "Landed block count or body hash mismatch"),
        (status = 403, description = "Caller cannot modify this resource"),
    )
)]
pub async fn finalize_handler(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<FinalizePayload>,
) -> ApiResult<StatusCode> {
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    backend
        .finalize_ingest(ResourceId::from(resource_id), payload, surface)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    operation_id = "list_blocks",
    path = "/api/resources/{id}/blocks",
    tag = "Ingest",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Currently-landed segment set (the resume/progress read)", body = BlocksResponse),
        (status = 403, description = "Caller cannot modify this resource"),
    )
)]
pub async fn list_blocks_handler(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<BlocksResponse>> {
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .list_blocks(ResourceId::from(resource_id))
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}
