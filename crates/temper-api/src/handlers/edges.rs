use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::backend::DbBackend;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::edge_service;
use crate::state::AppState;
use temper_core::types::ids::{EdgeId, ProfileId};
use temper_core::types::relationship_requests::{
    AssertRelationshipRequest, FoldRelationshipRequest, RelationshipAck, RetypeRelationshipRequest,
    ReweightRelationshipRequest,
};
use temper_workflow::operations::{
    AssertRelationship, Backend, FoldRelationship, RetypeRelationship, ReweightRelationship,
    Surface,
};
use temper_workflow::types::graph::GraphEdgeRow;

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
    Json(req): Json<AssertRelationshipRequest>,
) -> ApiResult<Json<RelationshipAck>> {
    let cmd = AssertRelationship {
        source: req.source,
        target: req.target,
        edge_kind: req.edge_kind,
        polarity: req.polarity,
        label: req.label,
        weight: req.weight,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .assert_relationship(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(RelationshipAck {
        edge_handle: Uuid::from(out.value),
    }))
}

#[utoipa::path(
    post,
    path = "/api/relationships/{edge_handle}/retype",
    tag = "Relationships",
    params(("edge_handle" = Uuid, Path, description = "Relationship edge handle")),
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
    Path(edge_handle): Path<Uuid>,
    Json(req): Json<RetypeRelationshipRequest>,
) -> ApiResult<Json<RelationshipAck>> {
    let cmd = RetypeRelationship {
        edge_handle: EdgeId::from(edge_handle),
        edge_kind: req.edge_kind,
        polarity: req.polarity,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .retype_relationship(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(RelationshipAck {
        edge_handle: Uuid::from(out.value),
    }))
}

#[utoipa::path(
    post,
    path = "/api/relationships/{edge_handle}/reweight",
    tag = "Relationships",
    params(("edge_handle" = Uuid, Path, description = "Relationship edge handle")),
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
    Path(edge_handle): Path<Uuid>,
    Json(req): Json<ReweightRelationshipRequest>,
) -> ApiResult<Json<RelationshipAck>> {
    let cmd = ReweightRelationship {
        edge_handle: EdgeId::from(edge_handle),
        weight: req.weight,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .reweight_relationship(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(RelationshipAck {
        edge_handle: Uuid::from(out.value),
    }))
}

#[utoipa::path(
    post,
    path = "/api/relationships/{edge_handle}/fold",
    tag = "Relationships",
    params(("edge_handle" = Uuid, Path, description = "Relationship edge handle")),
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
    Path(edge_handle): Path<Uuid>,
    Json(req): Json<FoldRelationshipRequest>,
) -> ApiResult<Json<RelationshipAck>> {
    let cmd = FoldRelationship {
        edge_handle: EdgeId::from(edge_handle),
        reason: req.reason,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .fold_relationship(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(RelationshipAck {
        edge_handle: Uuid::from(out.value),
    }))
}
