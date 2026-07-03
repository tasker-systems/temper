//! Knowledge-graph subgraph handler — serves `GET /api/graph/subgraph`.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::context_ref::parse_context_ref;
use temper_core::types::graph_atlas::{AtlasSubgraph, SliceRequest};
use temper_core::types::ids::ProfileId;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::services::context_service::resolve_context_ref;
use temper_services::services::graph_service::{
    self, aggregator_subgraph, AggregatorSubgraphParams,
};
use temper_services::state::AppState;
use temper_workflow::frontmatter::document::DocType;
use temper_workflow::types::graph::SubgraphResponse;

/// Query parameters for `GET /api/graph/subgraph`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SubgraphQuery {
    /// Context ref in decorated form: `@me/<slug>`, `@<handle>/<slug>`, `+<team-slug>/<slug>`,
    /// or a bare UUID. Bare context names are rejected — use the decorated form.
    pub context_ref: String,
}

#[utoipa::path(
    get,
    path = "/api/graph/subgraph",
    tag = "Graph",
    params(SubgraphQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Concept-centric subgraph", body = SubgraphResponse),
        (status = 400, description = "Bad query parameters", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Forbidden — caller is not a member of the requested team context", body = ErrorBody),
        (status = 404, description = "Context not found or not visible to caller", body = ErrorBody),
    )
)]
pub async fn get_subgraph(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<SubgraphQuery>,
) -> ApiResult<Json<SubgraphResponse>> {
    let cref =
        parse_context_ref(&query.context_ref).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let principal = ProfileId::from(auth.0.profile.id);
    let context_id = resolve_context_ref(&state.pool, principal, &cref).await?;

    let response = aggregator_subgraph(
        &state.pool,
        AggregatorSubgraphParams {
            caller_profile_id: auth.0.profile.id,
            context_id: *context_id,
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await?;

    Ok(Json(response))
}

/// POST /api/teams/{id}/graph/slice — R4 team-scoped parameterized neighborhood slice.
#[utoipa::path(
    post,
    path = "/api/teams/{id}/graph/slice",
    tag = "Graph",
    params(("id" = Uuid, Path, description = "Team id to scope the slice to")),
    request_body = SliceRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Neighborhood slice", body = AtlasSubgraph),
        (status = 400, description = "Empty seed set"),
        (status = 404, description = "Team not viewable by this profile")
    )
)]
pub async fn neighborhood_slice(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(req): Json<SliceRequest>,
) -> ApiResult<Json<AtlasSubgraph>> {
    graph_service::neighborhood_slice(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        req,
    )
    .await
    .map(Json)
}
