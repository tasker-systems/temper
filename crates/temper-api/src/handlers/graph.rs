//! Knowledge-graph subgraph handler — serves `GET /api/graph/subgraph`.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::context_ref::parse_context_ref;
use temper_core::types::graph_atlas::{AtlasSubgraph, SliceRequest};
use temper_core::types::graph_home::AtlasHome;
use temper_core::types::graph_territory::{TerritoryOverview, TerritorySlice};
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

/// Query parameters for `GET /api/teams/{id}/graph/territories`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct TerritoryQuery {
    /// Optional lens override; defaults to the global `telos-default` lens.
    pub lens_id: Option<Uuid>,
}

/// GET /api/teams/{id}/graph/territories — R2 Tier-0 panorama.
#[utoipa::path(
    get,
    path = "/api/teams/{id}/graph/territories",
    tag = "Graph",
    params(("id" = Uuid, Path, description = "Team id"), TerritoryQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Territory overview", body = TerritoryOverview),
        (status = 404, description = "Team not viewable by this profile")
    )
)]
pub async fn territory_overview(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Query(q): Query<TerritoryQuery>,
) -> ApiResult<Json<TerritoryOverview>> {
    graph_service::territory_overview(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        q.lens_id,
    )
    .await
    .map(Json)
}

/// Query parameters for `GET /api/graph/cogmaps/{id}/panorama`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct CogmapPanoramaQuery {
    /// Optional lens override; defaults to the cogmap's primary lens.
    pub lens_id: Option<Uuid>,
}

/// GET /api/graph/cogmaps/{id}/panorama — enter-a-cogmap Tier-0 interior.
#[utoipa::path(
    get,
    path = "/api/graph/cogmaps/{id}/panorama",
    tag = "Graph",
    params(("id" = Uuid, Path, description = "Cogmap id"), CogmapPanoramaQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Cogmap panorama", body = TerritoryOverview),
        (status = 404, description = "Cogmap not readable by this profile")
    )
)]
pub async fn cogmap_panorama(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Query(q): Query<CogmapPanoramaQuery>,
) -> ApiResult<Json<TerritoryOverview>> {
    graph_service::cogmap_panorama(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
        q.lens_id,
    )
    .await
    .map(Json)
}

/// GET /api/graph/regions/{region_id}/slice — R3 Tier-1 territory drill-in.
#[utoipa::path(
    get,
    path = "/api/graph/regions/{region_id}/slice",
    tag = "Graph",
    params(("region_id" = Uuid, Path, description = "Region id to slice")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Territory slice", body = TerritorySlice),
        (status = 404, description = "Region not readable by this profile")
    )
)]
pub async fn territory_slice(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(region_id): Path<Uuid>,
) -> ApiResult<Json<TerritorySlice>> {
    graph_service::territory_slice(&state.pool, ProfileId::from(auth.0.profile.id), region_id)
        .await
        .map(Json)
}

/// GET /api/graph/home — the you→teams→cogmaps membership home.
#[utoipa::path(
    get,
    path = "/api/graph/home",
    tag = "Graph",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Atlas membership home", body = AtlasHome))
)]
pub async fn atlas_home(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<AtlasHome>> {
    graph_service::atlas_home(&state.pool, ProfileId::from(auth.0.profile.id))
        .await
        .map(Json)
}
