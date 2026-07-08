//! Knowledge-graph subgraph handler — serves `GET /api/graph/subgraph`.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::context_ref::parse_context_ref;
use temper_core::types::graph_atlas::{AtlasSubgraph, SliceRequest};
use temper_core::types::graph_home::AtlasHome;
use temper_core::types::graph_territory::TerritoryOverview;
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

/// POST /api/cogmaps/{id}/graph/slice — R4 cogmap-scoped neighborhood slice.
#[utoipa::path(
    post,
    path = "/api/cogmaps/{id}/graph/slice",
    tag = "Graph",
    params(("id" = Uuid, Path, description = "Cogmap id to scope the slice to")),
    request_body = SliceRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Cogmap neighborhood slice", body = AtlasSubgraph),
        (status = 400, description = "Empty seed set"),
        (status = 404, description = "Cogmap not readable by this profile")
    )
)]
pub async fn cogmap_neighborhood_slice(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Json(req): Json<SliceRequest>,
) -> ApiResult<Json<AtlasSubgraph>> {
    graph_service::cogmap_neighborhood_slice(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
        req,
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

/// Query parameters for `GET /api/graph/regions/composition`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct RegionCompositionQuery {
    /// Comma-separated region ids — one region, or a shift-selected union.
    pub ids: String,
    /// Composition depth; defaults to 1, clamped to 3 by the service.
    pub depth: Option<i32>,
}

/// GET /api/graph/regions/composition — Beat D region→resources composition drill.
#[utoipa::path(
    get,
    path = "/api/graph/regions/composition",
    tag = "Graph",
    params(RegionCompositionQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Region composition subgraph (facets + linked context-resources)", body = AtlasSubgraph),
        (status = 400, description = "Malformed or empty region id list"),
        (status = 404, description = "A requested region is not readable by this profile")
    )
)]
pub async fn region_composition(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<RegionCompositionQuery>,
) -> ApiResult<Json<AtlasSubgraph>> {
    let ids: Vec<Uuid> = q
        .ids
        .split(',')
        .filter(|s| !s.is_empty())
        .map(Uuid::parse_str)
        .collect::<Result<_, _>>()
        .map_err(|e| ApiError::BadRequest(format!("invalid region id: {e}")))?;
    graph_service::region_composition_slice(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        &ids,
        q.depth.unwrap_or(1),
    )
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
