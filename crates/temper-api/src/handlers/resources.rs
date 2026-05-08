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

use temper_core::operations::{Backend, CreateResource, Surface};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::managed_meta::ManagedMeta;
use temper_core::types::resource::{ContentResponse, DeleteResponse};

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
        (status = 200, description = "Paginated list of visible resources with facets", body = ResourceListResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ResourceListParams>,
) -> ApiResult<Json<ResourceListResponse>> {
    resource_service::list_visible(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
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
    resource_service::get_visible(&state.pool, auth.0.profile.id, resource_id)
        .await
        .map(Json)
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
        (status = 400, description = "Partial body trio", body = ErrorBody),
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
    // Body trio is all-or-nothing.
    let body_fields_present = [
        req.content.is_some(),
        req.content_hash.is_some(),
        req.chunks_packed.is_some(),
    ];
    if body_fields_present.iter().any(|&p| p) && !body_fields_present.iter().all(|&p| p) {
        return Err(ApiError::BadRequest(
            "content, content_hash, and chunks_packed must all be present together or all be absent".to_string(),
        ));
    }

    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());

    resource_service::update(&state.pool, auth.0.profile.id, resource_id, &device_id, req)
        .await
        .map(Json)
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
    resource_service::delete(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        ResourceId::from(resource_id),
        &device_id,
    )
    .await?;
    Ok(Json(DeleteResponse { deleted: true }))
}
