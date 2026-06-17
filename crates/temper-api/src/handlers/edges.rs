use axum::extract::{Path, State};
use axum::Extension;
use axum::Json;
use uuid::Uuid;

use crate::backend::select_backend;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::auth::{AuthUser, DeviceId};
use crate::services::edge_service;
use crate::state::AppState;
use temper_core::operations::{
    AssertRelationship, FoldRelationship, RetypeRelationship, ReweightRelationship, Surface,
};
use temper_core::types::graph::GraphEdgeRow;
use temper_core::types::ids::ProfileId;
use temper_core::types::relationship_requests::{
    AssertRelationshipRequest, FoldRelationshipRequest, RelationshipAck, RetypeRelationshipRequest,
    ReweightRelationshipRequest,
};

// ─── Handlers ────────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/api/resources/{id}/edges",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Resource edges", body = Vec<GraphEdgeRow>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<Vec<GraphEdgeRow>>> {
    edge_service::list_resource_edges(&state.pool, auth.0.profile.id, resource_id)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/relationships",
    tag = "Relationships",
    security(("bearer_auth" = [])),
    request_body = AssertRelationshipRequest,
    responses(
        (status = 200, description = "Relationship asserted", body = RelationshipAck),
        (status = 400, description = "Invalid label or payload", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Cannot modify source resource", body = ErrorBody),
        (status = 404, description = "Source resource not found", body = ErrorBody),
    )
)]
pub async fn assert(
    State(state): State<AppState>,
    auth: AuthUser,
    device_id: Option<Extension<DeviceId>>,
    Json(req): Json<AssertRelationshipRequest>,
) -> ApiResult<Json<RelationshipAck>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());
    let cmd = AssertRelationship {
        source: req.source,
        target: req.target,
        edge_kind: req.edge_kind,
        polarity: req.polarity,
        label: req.label,
        weight: req.weight,
        origin: Surface::ApiHttp,
    };
    let backend = select_backend(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        device_id,
        Surface::ApiHttp,
    )
    .map_err(ApiError::from)?;
    let out = backend
        .assert_relationship(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(RelationshipAck {
        correlation_id: out.value,
    }))
}

#[utoipa::path(
    post,
    path = "/api/relationships/{correlation_id}/retype",
    tag = "Relationships",
    params(("correlation_id" = Uuid, Path, description = "Relationship correlation ID")),
    security(("bearer_auth" = [])),
    request_body = RetypeRelationshipRequest,
    responses(
        (status = 200, description = "Relationship retyped", body = RelationshipAck),
        (status = 400, description = "Invalid payload", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Cannot modify source resource", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody),
    )
)]
pub async fn retype(
    State(state): State<AppState>,
    auth: AuthUser,
    device_id: Option<Extension<DeviceId>>,
    Path(correlation_id): Path<Uuid>,
    Json(req): Json<RetypeRelationshipRequest>,
) -> ApiResult<Json<RelationshipAck>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());
    let cmd = RetypeRelationship {
        correlation_id,
        edge_kind: req.edge_kind,
        polarity: req.polarity,
        origin: Surface::ApiHttp,
    };
    let backend = select_backend(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        device_id,
        Surface::ApiHttp,
    )
    .map_err(ApiError::from)?;
    let out = backend
        .retype_relationship(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(RelationshipAck {
        correlation_id: out.value,
    }))
}

#[utoipa::path(
    post,
    path = "/api/relationships/{correlation_id}/reweight",
    tag = "Relationships",
    params(("correlation_id" = Uuid, Path, description = "Relationship correlation ID")),
    security(("bearer_auth" = [])),
    request_body = ReweightRelationshipRequest,
    responses(
        (status = 200, description = "Relationship reweighted", body = RelationshipAck),
        (status = 400, description = "Invalid payload", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Cannot modify source resource", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody),
    )
)]
pub async fn reweight(
    State(state): State<AppState>,
    auth: AuthUser,
    device_id: Option<Extension<DeviceId>>,
    Path(correlation_id): Path<Uuid>,
    Json(req): Json<ReweightRelationshipRequest>,
) -> ApiResult<Json<RelationshipAck>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());
    let cmd = ReweightRelationship {
        correlation_id,
        weight: req.weight,
        origin: Surface::ApiHttp,
    };
    let backend = select_backend(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        device_id,
        Surface::ApiHttp,
    )
    .map_err(ApiError::from)?;
    let out = backend
        .reweight_relationship(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(RelationshipAck {
        correlation_id: out.value,
    }))
}

#[utoipa::path(
    post,
    path = "/api/relationships/{correlation_id}/fold",
    tag = "Relationships",
    params(("correlation_id" = Uuid, Path, description = "Relationship correlation ID")),
    security(("bearer_auth" = [])),
    request_body = FoldRelationshipRequest,
    responses(
        (status = 200, description = "Relationship folded", body = RelationshipAck),
        (status = 400, description = "Invalid payload", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Cannot modify source resource", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody),
    )
)]
pub async fn fold(
    State(state): State<AppState>,
    auth: AuthUser,
    device_id: Option<Extension<DeviceId>>,
    Path(correlation_id): Path<Uuid>,
    Json(req): Json<FoldRelationshipRequest>,
) -> ApiResult<Json<RelationshipAck>> {
    let device_id = device_id
        .map(|d| d.0 .0.clone())
        .unwrap_or_else(|| "api".to_string());
    let cmd = FoldRelationship {
        correlation_id,
        reason: req.reason,
        origin: Surface::ApiHttp,
    };
    let backend = select_backend(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        device_id,
        Surface::ApiHttp,
    )
    .map_err(ApiError::from)?;
    let out = backend
        .fold_relationship(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(RelationshipAck {
        correlation_id: out.value,
    }))
}
