use axum::extract::{Path, Query, State};
use axum::Extension;
use axum::Json;
use uuid::Uuid;

use crate::backend::DbBackend;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::auth::{AuthUser, DeviceId};
use crate::services::resource_service::{
    self, ResolveByUriParams, ResourceCreateRequest, ResourceListParams, ResourceListResponse,
    ResourceRow, ResourceUpdateRequest,
};
use crate::services::{context_service, ingest_service};
use crate::state::AppState;

use temper_core::operations::{Backend, CreateResource, DeleteResource, ResourceRef, Surface};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::managed_meta::{ManagedMeta, ResourceMetaListResponse};
use temper_core::types::resource::{ContentResponse, DeleteResponse};

/// Combined response for `GET /api/resources`.
///
/// Returned shape depends on the `meta_only` query parameter. utoipa
/// represents this as `oneOf<ResourceListResponse, ResourceMetaListResponse>`.
#[derive(serde::Serialize, utoipa::ToSchema)]
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

/// Derive a URL-safe slug from a title.
///
/// Lowercases, replaces non-alphanumeric chars with hyphens, collapses runs
/// of hyphens, and strips leading/trailing hyphens. Matches the pattern
/// enforced by doc-type schema validation (`^[a-z0-9][a-z0-9-]*$`).
fn slugify_title(title: &str) -> String {
    let raw: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    raw.split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
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
        let response =
            resource_service::list_visible_meta(&state.pool, auth.0.profile.id, params).await?;
        Ok(ListResourcesResponse::Meta(response))
    } else {
        let response =
            resource_service::list_visible(&state.pool, auth.0.profile.id, params).await?;
        Ok(ListResourcesResponse::Default(response))
    }
}

#[utoipa::path(
    get,
    path = "/api/resources/by-uri",
    tag = "Resources",
    params(ResolveByUriParams),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Resolved resource", body = ResourceRow),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn by_uri(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ResolveByUriParams>,
) -> ApiResult<Json<ResourceRow>> {
    resource_service::resolve_by_uri(&state.pool, auth.0.profile.id, &params)
        .await
        .map(Json)
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
    use temper_core::operations::{Backend, ResourceRef, ShowResource, Surface};
    use temper_core::types::ids::ResourceId;

    let cmd = ShowResource {
        resource: ResourceRef::Uuid {
            id: ResourceId::from(resource_id),
        },
        origin: Surface::ApiHttp,
    };
    // Reads don't write audit, so device_id is "api" (not threaded through).
    let backend = DbBackend::new(
        state.pool.clone(),
        ProfileId::from(auth.0.profile.id),
        "api".to_string(),
        Surface::ApiHttp,
    );
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
    resource_service::get_content(&state.pool, auth.0.profile.id, resource_id)
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
    device_id: Option<Extension<DeviceId>>,
    Json(req): Json<ResourceCreateRequest>,
) -> ApiResult<Json<ResourceRow>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());

    // Resolve IDs → names for the operations command.
    // The visibility gate is enforced downstream by ingest_service::ingest
    // (via context_service::resolve_by_name which uses contexts_visible_to).
    let context_name = context_service::resolve_name_by_id(&state.pool, req.kb_context_id).await?;
    let doc_type_name =
        ingest_service::resolve_doc_type_name_by_id(&state.pool, req.kb_doc_type_id).await?;

    // When slug is absent, derive one from the title so the ingest path's
    // managed_meta validation (pattern ^[a-z0-9][a-z0-9-]*$) passes.
    let slug = req.slug.unwrap_or_else(|| slugify_title(&req.title));

    let cmd = CreateResource {
        context: context_name,
        doctype: doc_type_name,
        slug,
        title: req.title,
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(req.origin_uri),
        // POST /api/resources is a metadata-only create (no body); chunks are
        // produced by a follow-up async ingest job.
        chunks_packed: None,
        content_hash: None,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(
        state.pool.clone(),
        auth.0.profile.id.into(),
        device_id,
        Surface::ApiHttp,
    );
    let out: temper_core::operations::CommandOutput<ResourceRow> =
        backend.create_resource(cmd).await.map_err(ApiError::from)?;
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
    device_id: Option<Extension<DeviceId>>,
    Path(resource_id): Path<Uuid>,
    Json(req): Json<ResourceUpdateRequest>,
) -> ApiResult<Json<ResourceRow>> {
    use temper_core::operations::{BodyUpdate, ResourceRef, UpdateResource};
    use temper_core::types::ids::ResourceId;

    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());

    // Wire-supplied content_hash and chunks_packed are intentionally ignored —
    // the server is the single source of truth for body-trio derivation. Clients
    // should send content only; the translator (prepare_body_trio) recomputes
    // hash + chunks server-side. (Contract tightening from Phase 3b.)
    let body = req.content.map(BodyUpdate::new);

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

    let cmd = UpdateResource {
        resource: ResourceRef::Uuid {
            id: ResourceId::from(resource_id),
        },
        body,
        managed_meta,
        open_meta: req.open_meta,
        move_to: None,
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
    device_id: Option<Extension<DeviceId>>,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<DeleteResponse>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());

    let cmd = DeleteResource {
        resource: ResourceRef::Uuid {
            id: ResourceId::from(resource_id),
        },
        force: false,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(
        state.pool.clone(),
        ProfileId::from(auth.0.profile.id),
        device_id,
        Surface::ApiHttp,
    );
    backend.delete_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(DeleteResponse { deleted: true }))
}
