use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use crate::backend::DbBackend;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::context_service;
use crate::services::resource_service::{
    ResourceCreateRequest, ResourceListParams, ResourceListResponse, ResourceRow,
    ResourceUpdateRequest,
};
use crate::state::AppState;

use temper_core::context_ref::ContextRef;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_workflow::operations::{Backend, CreateResource, DeleteResource, Surface};
use temper_workflow::types::managed_meta::{ManagedMeta, ResourceMetaListResponse};
use temper_workflow::types::resource::{ContentResponse, DeleteResponse};

/// Combined response for `GET /api/resources`.
///
/// Returned shape depends on the `meta_only` query parameter. utoipa
/// represents this as `oneOf<ResourceListResponse, ResourceMetaListResponse>`.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum ListResourcesResponse {
    Default(ResourceListResponse),
    Meta(ResourceMetaListResponse),
}

impl axum::response::IntoResponse for ListResourcesResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::Default(r) => axum::Json(r).into_response(),
            Self::Meta(r) => axum::Json(r).into_response(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/resources",
    tag = "Resources",
    params(ResourceListParams),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Paginated list of visible resources with facets, or meta-only rows when meta_only=true", body = ListResourcesResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ResourceListParams>,
) -> ApiResult<ListResourcesResponse> {
    if params.meta_only.unwrap_or(false) {
        let response = crate::backend::substrate_read::list_meta_select(
            &state.pool,
            ProfileId::from(auth.0.profile.id),
            params,
        )
        .await?;
        Ok(ListResourcesResponse::Meta(response))
    } else {
        let response = crate::backend::substrate_read::list_select(
            &state.pool,
            ProfileId::from(auth.0.profile.id),
            params,
        )
        .await?;
        Ok(ListResourcesResponse::Default(response))
    }
}

#[utoipa::path(
    get,
    path = "/api/resources/{id}",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Resource metadata", body = ResourceRow),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<ResourceRow>> {
    use temper_core::types::ids::ResourceId;
    use temper_workflow::operations::{ShowResource, Surface};

    let cmd = ShowResource {
        resource: ResourceId::from(resource_id),
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend.show_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}

#[utoipa::path(
    get,
    path = "/api/resources/{id}/content",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Reconstituted markdown content", body = ContentResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn get_content(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<ContentResponse>> {
    crate::backend::substrate_read::get_content_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        ResourceId::from(resource_id),
    )
    .await
    .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/resources",
    tag = "Resources",
    request_body = ResourceCreateRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Created resource", body = ResourceRow),
        (status = 400, description = "Unknown context or doc_type ID", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Context not visible to profile", body = ErrorBody),
        (status = 409, description = "Conflict", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ResourceCreateRequest>,
) -> ApiResult<Json<ResourceRow>> {
    // Resolve the context UUID from the request — visibility-gated to the principal.
    // `ContextRef::Id` does the profile-visibility check without needing a name lookup.
    let context = context_service::resolve_context_ref(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        &ContextRef::Id(req.kb_context_id),
    )
    .await?;

    // When slug is absent, derive one from the title so the create path's
    // managed_meta validation (pattern ^[a-z0-9][a-z0-9-]*$) passes.
    let slug = req
        .slug
        .unwrap_or_else(|| temper_substrate::text::slugify(&req.title));

    let cmd = CreateResource {
        context,
        doctype: req.doc_type,
        slug,
        title: req.title,
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(req.origin_uri),
        // POST /api/resources is a metadata-only create: ResourceCreateRequest carries no
        // body, so there are no client chunks to honor here. The body-bearing (client-chunked)
        // create path is POST /api/ingest, which threads payload.chunks_packed through.
        chunks_packed: None,
        content_hash: None,
        act: Default::default(),
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend.create_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}

#[utoipa::path(
    patch,
    path = "/api/resources/{id}",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    request_body = ResourceUpdateRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Updated resource", body = ResourceRow),
        (status = 400, description = "Bad request (e.g. unknown open_meta key, or content sent without server-side pipeline)", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Forbidden", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(req): Json<ResourceUpdateRequest>,
) -> ApiResult<Json<ResourceRow>> {
    use temper_core::context_ref::parse_context_ref;
    use temper_core::types::ids::ResourceId;
    use temper_workflow::operations::{BodyUpdate, MoveSpec, UpdateResource};

    // Client-supplied chunks_packed (+ content_hash) are HONORED: the client did the
    // extract→chunk→embed locally, so the server carries them verbatim and only embeds
    // server-side as a fallback when they are absent. (Reverses PR#71's discard contract.)
    let body = req.content.map(|content| BodyUpdate {
        content,
        content_hash: req.content_hash,
        chunks_packed: req.chunks_packed,
    });

    // Fold top-level title/slug into managed_meta so the translator can extract
    // them uniformly. Only materialise Some(managed) when there's actually
    // something to fold (avoids routing a no-op through the meta branch).
    let managed_meta = match (req.title, req.slug, req.managed_meta) {
        (None, None, m) => m,
        (t, s, m) => {
            let mut merged = m.unwrap_or_default();
            if t.is_some() {
                merged.title = t;
            }
            if s.is_some() {
                merged.slug = s;
            }
            Some(merged)
        }
    };

    // Resolve context_to ref (if present) to a ContextId, gated by principal visibility.
    // parse_context_ref rejects bare names → ApiError::BadRequest (Decision 1).
    let move_to = if let Some(ref ctx_ref) = req.context_to {
        let r = parse_context_ref(ctx_ref).map_err(|e| ApiError::BadRequest(e.to_string()))?;
        let context_id = context_service::resolve_context_ref(
            &state.pool,
            ProfileId::from(auth.0.profile.id),
            &r,
        )
        .await?;
        Some(MoveSpec {
            context_to: Some(context_id),
            type_to: None,
        })
    } else {
        None
    };

    let cmd = UpdateResource {
        resource: ResourceId::from(resource_id),
        body,
        managed_meta,
        open_meta: req.open_meta,
        move_to,
        context_ref: None,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend.update_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}

#[utoipa::path(
    delete,
    path = "/api/resources/{id}",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Resource deleted", body = DeleteResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Forbidden", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<DeleteResponse>> {
    let cmd = DeleteResource {
        resource: ResourceId::from(resource_id),
        force: false,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    backend.delete_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(DeleteResponse { deleted: true }))
}
