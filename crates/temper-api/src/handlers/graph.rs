//! Knowledge-graph subgraph handler — serves `GET /api/graph/subgraph`.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::context_ref::parse_context_ref;
use temper_core::types::graph_atlas::{AtlasSubgraph, SliceRequest};
use temper_core::types::graph_context::ContextPanorama;
use temper_core::types::graph_home::AtlasHome;
use temper_core::types::graph_territory::TerritoryOverview;
use temper_core::types::ids::ProfileId;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::services::context_graph_service::{self, ResidualMemberQuery};
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

// ─── Beat E: the context door (panorama + composition) ──────────────────────────

/// Container-walk depth that defines container membership — and therefore which resources
/// count as "already contained" and are excluded from the residual buckets. The composition
/// drill reuses this (NOT its own `depth`) when resolving a bucket's seeds, so a bucket drill
/// reproduces exactly the residual set the panorama displayed. Matches the panorama default.
const CONTAINER_WALK_DEPTH: i32 = 2;

/// Container doc-types for a context walk: comma-split, defaulting to `["goal"]` (spec D4 — a
/// parameter, never a constant). An absent or blank value falls back to the default rather than
/// yielding zero containers.
fn parse_container_types(raw: Option<&str>) -> Vec<String> {
    let parsed: Vec<String> = raw
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    if parsed.is_empty() {
        vec!["goal".to_string()]
    } else {
        parsed
    }
}

/// Exactly one drill target for the composition endpoint — a container resource or a residual
/// bucket. Parsed up front (parse, don't validate) so the handler body never juggles two raw
/// `Option`s or re-checks the neither/both invariant.
#[derive(Debug)]
enum CompositionTarget {
    Container(Uuid),
    Bucket { key: String, value: String },
}

/// Decode the mutually-exclusive `container` / `group` query pair into a typed target. Rejects
/// *neither* and *both* with `BadRequest` — the shape is invalid, not the data.
fn parse_composition_target(
    container: Option<Uuid>,
    group: Option<&str>,
) -> ApiResult<CompositionTarget> {
    match (container, group) {
        (Some(id), None) => Ok(CompositionTarget::Container(id)),
        (None, Some(raw)) => {
            // Wire form `<group_key>:<group_value>`. A group value may itself contain a colon
            // (e.g. the stage bucket `in:progress`), so split on the FIRST colon only.
            let (key, value) = raw.split_once(':').ok_or_else(|| {
                ApiError::BadRequest("group must be `<group_key>:<group_value>`".into())
            })?;
            Ok(CompositionTarget::Bucket {
                key: key.to_string(),
                value: value.to_string(),
            })
        }
        (None, None) => Err(ApiError::BadRequest(
            "exactly one of `container` or `group` is required".into(),
        )),
        (Some(_), Some(_)) => Err(ApiError::BadRequest(
            "`container` and `group` are mutually exclusive".into(),
        )),
    }
}

/// Query parameters for `GET /api/graph/contexts/panorama`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ContextPanoramaQuery {
    /// Context ref in decorated form (`@me/<slug>`, `@<handle>/<slug>`, `+<team-slug>/<slug>`)
    /// or a bare UUID.
    pub context_ref: String,
    /// Property key the residual tray groups by. Defaults to `doc_type` (spec D2 — a
    /// parameter, not a constant).
    pub group_by: Option<String>,
    /// Comma-separated doc-types treated as containers. Defaults to `goal` (spec D4).
    pub container_types: Option<String>,
    /// Container-walk depth; defaults to 2, clamped to 3 by the SQL.
    pub depth: Option<i32>,
}

/// GET /api/graph/contexts/panorama — Beat E Tier-0: goal-container territories + residual tray.
#[utoipa::path(
    get,
    path = "/api/graph/contexts/panorama",
    tag = "Graph",
    params(ContextPanoramaQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Context panorama (container territories + residual tray)", body = ContextPanorama),
        (status = 400, description = "Bad query parameters", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Context not found or not visible to caller", body = ErrorBody),
    )
)]
pub async fn context_panorama(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ContextPanoramaQuery>,
) -> ApiResult<Json<ContextPanorama>> {
    let cref =
        parse_context_ref(&q.context_ref).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // Auth before reads: resolve gates visibility, yielding NotFound (404) — never a
    // Forbidden that would leak the context's existence — for a context the caller cannot see.
    let principal = ProfileId::from(auth.0.profile.id);
    let context_id = resolve_context_ref(&state.pool, principal, &cref).await?;

    let group_by = q.group_by.as_deref().unwrap_or("doc_type");
    let container_types = parse_container_types(q.container_types.as_deref());
    let depth = q.depth.unwrap_or(CONTAINER_WALK_DEPTH);

    context_graph_service::context_panorama(
        &state.pool,
        principal,
        context_id,
        group_by,
        &container_types,
        depth,
    )
    .await
    .map(Json)
}

/// Query parameters for `GET /api/graph/contexts/composition`. Exactly one of `container` /
/// `group` is required.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ContextCompositionQuery {
    /// Context ref (decorated or bare UUID) — the drill's home context.
    pub context_ref: String,
    /// Container resource id to drill.
    pub container: Option<Uuid>,
    /// Residual bucket to drill, as `<group_key>:<group_value>`.
    pub group: Option<String>,
    /// Comma-separated doc-types treated as containers. Defaults to `goal` (spec D4).
    pub container_types: Option<String>,
    /// Composition (drill) depth; defaults to 1, clamped to 3 by the service.
    pub depth: Option<i32>,
}

/// GET /api/graph/contexts/composition — Beat E Tier-1: the force-graph composition of a
/// container's (or a residual bucket's) members.
#[utoipa::path(
    get,
    path = "/api/graph/contexts/composition",
    tag = "Graph",
    params(ContextCompositionQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Composition subgraph for the drilled container or bucket", body = AtlasSubgraph),
        (status = 400, description = "Neither/both of container|group, or a malformed group", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Context not found or not visible to caller", body = ErrorBody),
    )
)]
pub async fn context_composition(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ContextCompositionQuery>,
) -> ApiResult<Json<AtlasSubgraph>> {
    let cref =
        parse_context_ref(&q.context_ref).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // Auth before reads — same deny-as-absence gate as the panorama.
    let principal = ProfileId::from(auth.0.profile.id);
    let context_id = resolve_context_ref(&state.pool, principal, &cref).await?;

    let target = parse_composition_target(q.container, q.group.as_deref())?;
    let container_types = parse_container_types(q.container_types.as_deref());
    let depth = q.depth.unwrap_or(1);

    // A container drill seeds with the single container id; a bucket drill resolves its member
    // ids first. The bucket's container-walk uses CONTAINER_WALK_DEPTH (the panorama's depth),
    // not the drill `depth`, so the seed set matches the bucket the panorama showed.
    let seeds: Vec<Uuid> = match target {
        CompositionTarget::Container(id) => vec![id],
        CompositionTarget::Bucket { key, value } => {
            context_graph_service::residual_member_ids(
                &state.pool,
                ResidualMemberQuery {
                    profile_id: principal,
                    context_id,
                    group_key: key.as_str(),
                    group_value: value.as_str(),
                    container_types: container_types.as_slice(),
                    depth: CONTAINER_WALK_DEPTH,
                },
            )
            .await?
        }
    };

    context_graph_service::context_composition(&state.pool, principal, &seeds, depth)
        .await
        .map(Json)
}
