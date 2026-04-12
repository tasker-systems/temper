use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::edge_service;
use crate::state::AppState;
use temper_core::types::graph::GraphEdgeRow;

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
