use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;
use temper_core::types::facet_requests::{
    EdgeFacetSetRequest, EdgeFacetsResponse, FacetAck, FacetSetRequest, ResourceFacetsResponse,
};
use temper_core::types::ids::{EdgeId, ProfileId, ResourceId};
use temper_core::types::property_owner::PropertyOwner;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;
use temper_workflow::operations::{Backend, SetFacet};

// ─── Handlers ────────────────────────────────────────────────────────────────

/// Set a facet on a resource
#[utoipa::path(
    post,
    path = "/api/facets",
    tag = "Facets",
    security(("bearer_auth" = [])),
    request_body = FacetSetRequest,
    responses(
        (status = 200, description = "Facet set", body = FacetAck),
        (status = 400, description = "Invalid payload", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Cannot modify resource", body = ErrorBody),
        (status = 404, description = "Resource not found", body = ErrorBody),
    )
)]
pub async fn set_facet(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Json(req): Json<FacetSetRequest>,
) -> ApiResult<Json<FacetAck>> {
    let act = req.act.into_act_context().map_err(ApiError::from)?;
    let cmd = SetFacet {
        owner: PropertyOwner::resource(ResourceId::from(req.resource)),
        values: req.values,
        weight: req.weight,
        act,
        origin: surface,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile().id));
    let out = backend.set_facet(cmd).await.map_err(ApiError::from)?;
    Ok(Json(FacetAck {
        property_ids: out.value.into_iter().map(Uuid::from).collect(),
    }))
}

// A separate route rather than a mode of `POST /api/facets`, for two reasons that are both
// about the owner not being a payload choice: the edge is addressed in the path (matching every
// other edge write — `/api/relationships/{edge_handle}/retype|reweight|fold`), and the
// authorization gate is a different question. `DbBackend::set_facet` dispatches on the typed
// owner to `check_edge_mutable`, which is the same gate that governs re-typing or folding the
// same edge.
//
// Both statuses are reachable, and they mean different things. 404 for an edge that does not
// exist, is folded, or whose target the caller cannot read — that last arm is `NotFound` rather
// than `Forbidden` on purpose, so the write never confirms the existence of a resource the
// caller has no standing to see. 403 for an edge the caller can legitimately see but may not
// author into: it fails source-write or container-write on the edge's home.
//
// An earlier version of this comment claimed the endpoint returns "404 rather than 403"
// outright, twelve lines above an OpenAPI block declaring `403`. The annotation was right and
// the prose was wrong; `check_edge_mutable` renders `Forbidden` for clauses 1 and 2 and
// `NotFound` for the row lookup and clause 3.
/// Set a facet on a relationship
///
/// Sets a facet whose owner is an edge rather than a resource. The edge is addressed in the path, matching the other edge writes (`retype`, `reweight`, `fold`).
///
/// Answers 404 when the edge does not exist, is folded, or has a target you cannot read. That last case answers 404 rather than 403 on purpose, so a refusal never confirms the existence of something you are not allowed to see.
#[utoipa::path(
    post,
    path = "/api/relationships/{edge_handle}/facets",
    tag = "Facets",
    params(("edge_handle" = Uuid, Path, description = "Relationship edge handle")),
    security(("bearer_auth" = [])),
    request_body = EdgeFacetSetRequest,
    responses(
        (status = 200, description = "Facet set on the edge", body = FacetAck),
        (status = 400, description = "Invalid payload", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Cannot modify this relationship", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody),
    )
)]
pub async fn set_edge_facet(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Path(edge_handle): Path<Uuid>,
    Json(req): Json<EdgeFacetSetRequest>,
) -> ApiResult<Json<FacetAck>> {
    let act = req.act.into_act_context().map_err(ApiError::from)?;
    let cmd = SetFacet {
        owner: PropertyOwner::edge(EdgeId::from(edge_handle)),
        values: req.values,
        weight: req.weight,
        act,
        origin: surface,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile().id));
    let out = backend.set_facet(cmd).await.map_err(ApiError::from)?;
    Ok(Json(FacetAck {
        property_ids: out.value.into_iter().map(Uuid::from).collect(),
    }))
}

// Read-side gate is `edges_visible_to` — see `edge_service::list_edge_facets`. Reads stay
// service-direct on both surfaces by design (the trait projections are lossy), so this does not
// route through the backend.
/// Read the live facets of one edge
///
/// One entry per assert, each with its weight and its author. Readable only if you can read the edge.
#[utoipa::path(
    get,
    path = "/api/relationships/{edge_handle}/facets",
    tag = "Facets",
    params(("edge_handle" = Uuid, Path, description = "Relationship edge handle")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The edge's live facets", body = EdgeFacetsResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody),
    )
)]
pub async fn list_edge_facets(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(edge_handle): Path<Uuid>,
) -> ApiResult<Json<EdgeFacetsResponse>> {
    let facets = temper_services::services::edge_service::list_edge_facets(
        &state.pool,
        auth.0.profile().id,
        edge_handle,
    )
    .await?;
    Ok(Json(EdgeFacetsResponse {
        edge_handle,
        facets,
    }))
}

// Read-side gate is `resources_visible_to`, asked via `readback::is_resource_visible` — see
// `facet_service::list_resource_facets` for why it is asked separately and first. Reads stay
// service-direct on both surfaces by design, so this does not route through the backend.
//
// The same argument `GET /api/resources/{id}/citation-audits` makes, and the same denial
// dialect.
//
// Path is `/api/resources/{id}/facets`, not a mode of `/api/facets`. The write posts to
// `/api/facets` with the resource in the body because a facet-set is an act on a payload; a
// read addresses a resource, so the resource belongs in the path alongside its siblings
// (`/citation-audits`, `/evidence`).
/// Read the live facets of one resource
///
/// One entry per assert, each with its weight and its author. This is the confirming read for a facet write, which steers region formation and graph grouping.
///
/// 200 with an empty list and 404 are different answers: empty means readable with nothing asserted; 404 means unreadable or absent, indistinguishably. The empty list is only reachable once the readability gate has passed, so it is not an existence oracle.
#[utoipa::path(
    get,
    operation_id = "list_resource_facets",
    path = "/api/resources/{id}/facets",
    tag = "Facets",
    params(("id" = Uuid, Path, description = "Resource ID whose facets are read")),
    security(("bearer_auth" = [])),
    responses(
        (
            status = 200,
            description = "The resource's live facets, one row per assert, oldest-first. Empty when the resource is readable but carries no facets.",
            body = ResourceFacetsResponse
        ),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (
            status = 404,
            description = "Not found — the resource is unreadable or does not exist (deliberately indistinguishable)",
            body = ErrorBody
        ),
    )
)]
pub async fn list_resource_facets(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ResourceFacetsResponse>> {
    let facets = temper_services::services::facet_service::list_resource_facets(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        ResourceId::from(id),
    )
    .await?;
    Ok(Json(ResourceFacetsResponse {
        resource: id,
        facets,
    }))
}
