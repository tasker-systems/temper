//! HTTP handlers for the segmented (multi-block) ingest surface: append one segment, finalize
//! the session, and read the currently-landed set back (the resume/progress query).
//!
//! Thin handlers only: `AnyPrincipal` extractor → `DbBackend::with_proof` → dispatch the `Backend` trait
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

use crate::middleware::auth::AnyPrincipal;
use crate::middleware::surface::RequestSurface;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;

use temper_core::types::ids::ResourceId;
use temper_core::types::ingest::{AppendBlockPayload, BlocksResponse, FinalizePayload};
use temper_workflow::operations::Backend;

/// Append a block to an in-progress ingest
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
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Caller cannot modify this resource"),
        (status = 409, description = "The ingest has ended (cancelled or abandoned; code INGEST_ENDED); not resumable — start a new upload", body = ErrorBody),
        (status = 410, description = "The resource was erased (code RESOURCE_ERASED); answered only to a caller who held standing on it, everyone else gets 403", body = ErrorBody),
    )
)]
pub async fn append_block_handler(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    RequestSurface(surface): RequestSurface,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<AppendBlockPayload>,
) -> ApiResult<Json<BlocksResponse>> {
    let backend = DbBackend::with_proof(state.pool.clone(), &auth.0);
    let out = backend
        .append_block(ResourceId::from(resource_id), payload, surface)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}

/// Finalize a segmented ingest
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
        (status = 400, description = "The path id is not a UUID, or the request body is not syntactically valid JSON (the extractor's plain-text rejection). A landed block count or body hash mismatch is the 409, never a 400"),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Caller cannot modify this resource"),
        (status = 409, description = "The landed block count or the body hash does not match what the caller declared (code CONFLICT; resumable — append the gap and finalize again); or the ingest has ended (cancelled or abandoned; code INGEST_ENDED), which is not resumable — start a new upload", body = ErrorBody),
        (status = 410, description = "The resource was erased (code RESOURCE_ERASED); answered only to a caller who held standing on it, everyone else gets 403", body = ErrorBody),
        (status = 422, description = "The stored bytes do not match the declared content hash (code CONTENT_INTEGRITY); not resumable — discard and re-upload", body = ErrorBody),
    )
)]
pub async fn finalize_handler(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    RequestSurface(surface): RequestSurface,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<FinalizePayload>,
) -> ApiResult<StatusCode> {
    let backend = DbBackend::with_proof(state.pool.clone(), &auth.0);
    backend
        .finalize_ingest(ResourceId::from(resource_id), payload, surface)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// List the blocks landed so far
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
    auth: AnyPrincipal,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<BlocksResponse>> {
    let backend = DbBackend::with_proof(state.pool.clone(), &auth.0);
    let out = backend
        .list_blocks(ResourceId::from(resource_id))
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}
