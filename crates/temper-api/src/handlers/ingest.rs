use axum::extract::{Path, State};
use axum::Extension;
use axum::Json;
use uuid::Uuid;

use crate::backend::DbBackend;
use crate::error::{ApiError, ApiResult};
use crate::middleware::auth::{AuthUser, DeviceId};
use crate::services::ingest_service;
use crate::state::AppState;

use temper_core::operations::{Backend, BodyUpdate, CreateResource, Surface};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::ingest::IngestPayload;
use temper_core::types::managed_meta::ManagedMeta;
use temper_core::types::resource::ResourceRow;

#[utoipa::path(
    post,
    path = "/api/ingest",
    tag = "Ingest",
    security(("bearer_auth" = [])),
    request_body = IngestPayload,
    responses(
        (status = 200, description = "Resource created (or existing on dedup)", body = ResourceRow),
        (status = 400, description = "Invalid payload"),
        (status = 404, description = "Context not found"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    device_id: Option<Extension<DeviceId>>,
    Json(payload): Json<IngestPayload>,
) -> ApiResult<Json<ResourceRow>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());

    // Convert IngestPayload's Option<Value> managed_meta to typed ManagedMeta.
    // Parse failures (malformed JSON for ManagedMeta shape) surface as BadRequest.
    let managed_meta: ManagedMeta = match payload.managed_meta {
        Some(v) => serde_json::from_value(v)
            .map_err(|e| ApiError::BadRequest(format!("invalid managed_meta: {e}")))?,
        None => ManagedMeta::default(),
    };

    let body = if payload.content.is_empty() {
        None
    } else {
        Some(BodyUpdate {
            content: payload.content,
        })
    };

    let cmd = CreateResource {
        context: payload.context_name,
        doctype: payload.doc_type_name,
        slug: payload.slug,
        title: payload.title,
        body,
        managed_meta,
        open_meta: payload.open_meta,
        origin_uri: Some(payload.origin_uri),
        // Forward caller-supplied chunks so ingest_service can skip the embed
        // pipeline when pre-computed chunks are present. content_hash is left
        // None — ingest_service recomputes it from content when the pipeline
        // runs, and leaves it absent (empty string stored) otherwise.
        chunks_packed: payload.chunks_packed,
        origin: Surface::ApiHttp,
    };

    let backend = DbBackend::new(
        state.pool.clone(),
        ProfileId::from(auth.0.profile.id),
        device_id,
        Surface::ApiHttp,
    );
    let out = backend.create_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}

#[utoipa::path(
    put,
    path = "/api/ingest/{id}",
    tag = "Ingest",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    request_body = IngestPayload,
    responses(
        (status = 200, description = "Resource updated", body = ResourceRow),
        (status = 400, description = "Invalid payload"),
        (status = 404, description = "Resource not found"),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    device_id: Option<Extension<DeviceId>>,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<IngestPayload>,
) -> ApiResult<Json<ResourceRow>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());
    ingest_service::update(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        ResourceId::from(resource_id),
        &device_id,
        payload,
    )
    .await
    .map(Json)
}
