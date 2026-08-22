use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::data_artifact::KindOwnerInput;
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest, ShapeView};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ShapeId};
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
    auth: AuthUser,
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
    auth: AuthUser,
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
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Json(req): Json<ShapeDeclareRequest>,
) -> ApiResult<Json<ShapeView>> {
    let profile = ProfileId::from(auth.0.profile().id);
    let home = AnchorRef::context(ContextId::from(context_id));

    let emitter = temper_substrate::writes::resolve_emitter(&state.pool, profile, "web")
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
