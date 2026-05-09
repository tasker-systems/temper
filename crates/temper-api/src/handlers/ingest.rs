use axum::extract::{Path, State};
use axum::Extension;
use axum::Json;
use uuid::Uuid;

use crate::backend::DbBackend;
use crate::error::{ApiError, ApiResult};
use crate::middleware::auth::{AuthUser, DeviceId};
use crate::state::AppState;

use temper_core::operations::{
    Backend, BodyUpdate, CreateResource, ResourceRef, Surface, UpdateResource,
};
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
        Some(BodyUpdate::new(payload.content))
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
        chunks_packed: payload.chunks_packed,
        content_hash: payload.content_hash,
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

    // Convert IngestPayload's Option<Value> managed_meta to typed ManagedMeta.
    let managed_meta: Option<ManagedMeta> = match payload.managed_meta {
        Some(v) => Some(
            serde_json::from_value(v)
                .map_err(|e| ApiError::BadRequest(format!("invalid managed_meta: {e}")))?,
        ),
        None => None,
    };

    let body = if payload.content.is_empty() {
        None
    } else {
        Some(BodyUpdate {
            content: payload.content,
            // Forward caller-supplied pre-computed chunks so the translator
            // skips prepare_body_trio (and the ONNX pipeline) when they are
            // present. Matches the short-circuit in ingest_service::update.
            content_hash: payload.content_hash,
            chunks_packed: payload.chunks_packed,
        })
    };

    let cmd = UpdateResource {
        resource: ResourceRef::Uuid {
            id: ResourceId::from(resource_id),
        },
        body,
        managed_meta,
        open_meta: payload.open_meta,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(
        state.pool.clone(),
        ProfileId::from(auth.0.profile.id),
        device_id,
        Surface::ApiHttp,
    );
    let out = backend.update_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}
