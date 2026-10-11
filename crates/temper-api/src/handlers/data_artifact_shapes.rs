use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AnyPrincipal;
use crate::middleware::surface::RequestSurface;
use temper_core::types::data_artifact::KindOwnerInput;
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest, ShapeView};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ContextId, ProfileId, ShapeId};
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::services::shape_service::{self, DeclareShapeServiceParams};
use temper_services::state::AppState;
use temper_substrate::payloads::{AnchorRef, KindOwner};

/// List live shapes declared for a context home.
///
/// Visibility-gated: the caller only sees shapes whose home anchor they can read.
/// Returns an empty set (never an error) for an unreadable context.
#[utoipa::path(
    get,
    operation_id = "list_shapes",
    path = "/api/contexts/{id}/shapes",
    tag = "Data Artifact Shapes",
    params(
        ("id" = Uuid, Path, description = "Context ID (the shape's home anchor)"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Live shapes declared for this context", body = Vec<ShapeView>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list_shapes(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(context_id): Path<Uuid>,
) -> ApiResult<Json<Vec<ShapeView>>> {
    let shapes = temper_services::backend::substrate_read::list_shapes(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        HomeAnchor::Context(ContextId::from(context_id)),
    )
    .await?;
    Ok(Json(shapes))
}

/// Get a single shape by ID.
///
/// Visibility-gated: returns 404 if the shape does not exist or its owning home
/// anchor is not readable to the caller. Includes folded shapes (audit/history).
#[utoipa::path(
    get,
    operation_id = "get_shape",
    path = "/api/shapes/{shape_id}",
    tag = "Data Artifact Shapes",
    params(
        ("shape_id" = Uuid, Path, description = "Shape ID"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The shape with its schema and enforcement mode", body = ShapeView),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found or not visible", body = ErrorBody),
    )
)]
pub async fn get_shape(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(shape_id): Path<Uuid>,
) -> ApiResult<Json<ShapeView>> {
    let shape = temper_services::backend::substrate_read::get_shape(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        ShapeId::from(shape_id),
    )
    .await?
    .ok_or_else(|| ApiError::NotFound("shape not found".to_string()))?;
    Ok(Json(shape))
}

/// Declare a shape for a data-artifact family within a context home.
///
/// Authority-gated: the caller must have authoring authority over the context
/// (`context_authorable_by_profile`). The service layer applies the gate before
/// any write — a caller who cannot author the home is refused with 403.
#[utoipa::path(
    post,
    operation_id = "declare_shape",
    path = "/api/contexts/{id}/shapes",
    tag = "Data Artifact Shapes",
    params(
        ("id" = Uuid, Path, description = "Context ID (the shape's home anchor)"),
    ),
    request_body = ShapeDeclareRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Declared shape", body = ShapeView),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "No authoring authority over the home context", body = ErrorBody),
        (status = 404, description = "Context not found", body = ErrorBody),
    )
)]
pub async fn declare_shape(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    RequestSurface(surface): RequestSurface,
    Path(context_id): Path<Uuid>,
    Json(req): Json<ShapeDeclareRequest>,
) -> ApiResult<Json<ShapeView>> {
    let profile = ProfileId::from(auth.0.profile().id);
    let home = AnchorRef::context(ContextId::from(context_id));

    // The emitter marker rides the REQUEST's resolved surface, not a literal: a plain web
    // caller resolves to `web` exactly as before (behavior-neutral), while an act forwarded
    // through the MCP relay stamps the caller's own `@mcp` — the attribution the direct
    // tool's in-process `resolve_emitter(…, "mcp")` used to carry.
    let emitter = temper_substrate::writes::resolve_emitter(&state.pool, profile, surface.marker())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let kind_owner = req.kind_owner.map(|ko| match ko {
        KindOwnerInput::Profile(id) => KindOwner::Profile(id),
        KindOwnerInput::Team(id) => KindOwner::Team(id),
    });

    let enforcement = match req.enforcement {
        EnforcementMode::Advisory => temper_substrate::payloads::EnforcementMode::Advisory,
        EnforcementMode::Enforcing => temper_substrate::payloads::EnforcementMode::Enforcing,
    };

    let shape_id = shape_service::declare_shape(
        &state.pool,
        DeclareShapeServiceParams {
            home,
            kind: &req.kind,
            kind_owner,
            schema: &req.schema,
            enforcement,
            principal: profile,
            emitter,
        },
    )
    .await?;

    let shape = temper_services::backend::substrate_read::get_shape(&state.pool, profile, shape_id)
        .await?
        .ok_or_else(|| ApiError::Internal("shape declared but not retrievable".to_string()))?;

    Ok(Json(shape))
}

/// List live shapes declared for a cognitive-map home.
///
/// The cogmap-home twin of [the context route](list_shapes): the MCP
/// `list_data_artifact_shapes` tool admits both home anchors (`home_type`:
/// `"context"` or `"cogmap"`) and the substrate read is home-generic, but until
/// this route the cogmap arm had no wire door. Visibility-gated the same way: the
/// caller only sees shapes whose home anchor they can read, and an unreadable map
/// answers an empty set (never an error).
#[utoipa::path(
    get,
    operation_id = "list_cogmap_shapes",
    path = "/api/cognitive-maps/{id}/shapes",
    tag = "Data Artifact Shapes",
    params(
        ("id" = Uuid, Path, description = "Cognitive map ID (the shape's home anchor)"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Live shapes declared for this cognitive map", body = Vec<ShapeView>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list_cogmap_shapes(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(cogmap_id): Path<Uuid>,
) -> ApiResult<Json<Vec<ShapeView>>> {
    let shapes = temper_services::backend::substrate_read::list_shapes(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        HomeAnchor::Cogmap(CogmapId::from(cogmap_id)),
    )
    .await?;
    Ok(Json(shapes))
}

/// Declare a shape for a data-artifact family within a cognitive-map home.
///
/// The cogmap-home twin of [the context route](declare_shape), added for the same
/// reason. Authority-gated: the caller must have authoring authority over the map
/// (`cogmap_authorable_by_profile`) — the service layer applies the gate before any
/// write, refusing with 403. The emitter marker rides the request's resolved
/// surface, so a shape declared through the MCP relay attributes to the caller's
/// own `@mcp` entity.
#[utoipa::path(
    post,
    operation_id = "declare_cogmap_shape",
    path = "/api/cognitive-maps/{id}/shapes",
    tag = "Data Artifact Shapes",
    params(
        ("id" = Uuid, Path, description = "Cognitive map ID (the shape's home anchor)"),
    ),
    request_body = ShapeDeclareRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Declared shape", body = ShapeView),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "No authoring authority over the home cognitive map", body = ErrorBody),
        (status = 404, description = "Cognitive map not found", body = ErrorBody),
    )
)]
pub async fn declare_cogmap_shape(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    RequestSurface(surface): RequestSurface,
    Path(cogmap_id): Path<Uuid>,
    Json(req): Json<ShapeDeclareRequest>,
) -> ApiResult<Json<ShapeView>> {
    let profile = ProfileId::from(auth.0.profile().id);
    let home = AnchorRef::cogmap(CogmapId::from(cogmap_id));

    let emitter = temper_substrate::writes::resolve_emitter(&state.pool, profile, surface.marker())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let kind_owner = req.kind_owner.map(|ko| match ko {
        KindOwnerInput::Profile(id) => KindOwner::Profile(id),
        KindOwnerInput::Team(id) => KindOwner::Team(id),
    });

    let enforcement = match req.enforcement {
        EnforcementMode::Advisory => temper_substrate::payloads::EnforcementMode::Advisory,
        EnforcementMode::Enforcing => temper_substrate::payloads::EnforcementMode::Enforcing,
    };

    let shape_id = shape_service::declare_shape(
        &state.pool,
        DeclareShapeServiceParams {
            home,
            kind: &req.kind,
            kind_owner,
            schema: &req.schema,
            enforcement,
            principal: profile,
            emitter,
        },
    )
    .await?;

    let shape = temper_services::backend::substrate_read::get_shape(&state.pool, profile, shape_id)
        .await?
        .ok_or_else(|| ApiError::Internal("shape declared but not retrievable".to_string()))?;

    Ok(Json(shape))
}
