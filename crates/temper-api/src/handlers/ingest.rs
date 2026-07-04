use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult};
use temper_services::state::AppState;

use temper_core::context_ref::parse_context_ref;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ProfileId, ResourceId};
use temper_core::types::ingest::IngestPayload;
use temper_workflow::operations::{Backend, BodyUpdate, CreateResource, Surface, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::ResourceRow;

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
    Json(payload): Json<IngestPayload>,
) -> ApiResult<Json<ResourceRow>> {
    let profile_id = ProfileId::from(auth.0.profile.id);

    // Resolve the home anchor — exactly one of a cognitive map or a context.
    // The cogmap branch takes precedence: when `home_cogmap_id` is set the home
    // is that map and `context_ref` is ignored.
    let home = match payload.home_cogmap_id {
        Some(map) => {
            // Auth before writes: the producer gate (a named service seam delegating to
            // team-cogmap membership) runs and denies BEFORE any home-row write.
            let cogmap = CogmapId::from(map);
            if !temper_services::services::cogmap_service::authorable_by_profile(
                &state.pool,
                profile_id,
                cogmap,
            )
            .await?
            {
                return Err(ApiError::Forbidden);
            }
            HomeAnchor::Cogmap(cogmap)
        }
        None => {
            // Parse the context ref string (UUID or @owner/slug). Bare names are rejected with 400.
            let cref = parse_context_ref(&payload.context_ref)
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            // Resolve to a ContextId, visibility-gated to the calling principal.
            let context = temper_services::services::context_service::resolve_context_ref(
                &state.pool,
                profile_id,
                &cref,
            )
            .await?;
            HomeAnchor::Context(context)
        }
    };

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
            // Create passes chunks/hash separately on CreateResource (below), so the
            // BodyUpdate carries only content + provenance sources here.
            content_hash: None,
            chunks_packed: None,
            sources: payload.sources,
            // Ingest create writes a single body block; per-block addressing is the PATCH surface.
            content_block: None,
        })
    };

    let act = payload.act.into_act_context().map_err(ApiError::from)?;

    let cmd = CreateResource {
        home,
        doctype: payload.doc_type_name,
        slug: payload.slug,
        title: payload.title,
        body,
        managed_meta,
        open_meta: payload.open_meta,
        origin_uri: Some(payload.origin_uri),
        chunks_packed: payload.chunks_packed,
        content_hash: payload.content_hash,
        act,
        origin: Surface::ApiHttp,
    };

    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
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
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<IngestPayload>,
) -> ApiResult<Json<ResourceRow>> {
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
            sources: payload.sources,
            // Ingest update revises the resource's sole body block; per-block addressing is the PATCH surface.
            content_block: None,
        })
    };

    let act = payload.act.into_act_context().map_err(ApiError::from)?;
    let cmd = UpdateResource {
        resource: ResourceId::from(resource_id),
        body,
        managed_meta,
        open_meta: payload.open_meta,
        move_to: None,
        context_ref: None,
        act,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend.update_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}
