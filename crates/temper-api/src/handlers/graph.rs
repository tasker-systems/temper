//! Knowledge-graph Atlas handlers — the cogmap slice/panorama, the Beat-D region
//! composition, the membership home, and the Beat-E context door (panorama + composition).

use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::context_ref::parse_context_ref;
use temper_core::types::graph_atlas::{AtlasEntry, AtlasSubgraph, SliceRequest};
use temper_core::types::graph_context::ContextPanorama;
use temper_core::types::graph_home::AtlasHome;
use temper_core::types::graph_territory::TerritoryOverview;
use temper_core::types::ids::ProfileId;
use temper_core::types::query_params::{
    CogmapPanoramaQuery, ContextCompositionQuery, ContextPanoramaQuery, EntryQuery,
    RegionCompositionQuery, TraverseQuery,
};
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::services::context_graph_service::{self, ResidualMemberQuery};
use temper_services::services::context_service::resolve_context_ref;
use temper_services::services::graph_service;
use temper_services::state::AppState;

/// Slice a neighborhood within a cognitive map
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
        ProfileId::from(auth.0.profile().id),
        cogmap_id,
        req,
    )
    .await
    .map(Json)
}

/// Read a cognitive map's interior
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
        ProfileId::from(auth.0.profile().id),
        cogmap_id,
        q.lens_id,
    )
    .await
    .map(Json)
}

/// Read the resources composing a region
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
        ProfileId::from(auth.0.profile().id),
        &ids,
        q.depth.unwrap_or(1),
    )
    .await
    .map(Json)
}

/// Read what your work is built around
///
/// The entry door for a reader who has asked nothing: the most-connected resources they can see,
/// plus every edge among them. Ranking and drawing use the same criterion, so no returned edge
/// points at a node that is not on the canvas. The response declares its own bounds, including how
/// many resources it did not draw for having no connections.
#[utoipa::path(
    get,
    path = "/api/graph/entry",
    tag = "Graph",
    params(EntryQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Orientation subgraph — top-K by degree with their induced edges and bounds", body = AtlasEntry),
        (status = 400, description = "Non-positive k, or a malformed anchor id", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<EntryQuery>,
) -> ApiResult<Json<AtlasEntry>> {
    let anchors: Vec<Uuid> = match q.places.as_deref() {
        None | Some("") => Vec::new(),
        Some(raw) => raw
            .split(',')
            .filter(|s| !s.is_empty())
            .map(Uuid::parse_str)
            .collect::<Result<_, _>>()
            .map_err(|e| ApiError::BadRequest(format!("invalid anchor id: {e}")))?,
    };
    graph_service::entry_orientation_slice(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        &anchors,
        q.k,
    )
    .await
    .map(Json)
}

/// Traverse from where you are
///
/// Moves inside a space that a question already set, without re-running the question. The other
/// half of *a composition grounds you; it does not navigate you* — grounding chooses the space,
/// this walks it.
#[utoipa::path(
    get,
    path = "/api/graph/traverse",
    tag = "Graph",
    params(TraverseQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The subgraph reached from the given nodes", body = AtlasSubgraph),
        (status = 400, description = "Malformed or empty node id list", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn traverse(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<TraverseQuery>,
) -> ApiResult<Json<AtlasSubgraph>> {
    let ids: Vec<Uuid> = q
        .from
        .split(',')
        .filter(|s| !s.is_empty())
        .map(Uuid::parse_str)
        .collect::<Result<_, _>>()
        .map_err(|e| ApiError::BadRequest(format!("invalid node id: {e}")))?;
    graph_service::traversal_slice(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        &ids,
        q.depth.unwrap_or(1),
    )
    .await
    .map(Json)
}

/// Read your teams and cognitive maps
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
    graph_service::atlas_home(&state.pool, ProfileId::from(auth.0.profile().id))
        .await
        .map(Json)
}

// ─── Beat E: the context door (panorama + composition) ──────────────────────────

/// Default container-walk depth. The walk defines container membership — and therefore which
/// resources count as "already contained" and are excluded from the residual buckets.
///
/// Both endpoints default to it, and a bucket drill must walk at the SAME depth the panorama
/// walked or it resolves seeds the tray never showed. That is why `container_depth` is a
/// parameter on the composition query rather than a constant read here: the caller echoes back
/// whatever depth it passed to the panorama.
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

/// Read goal-container territories and residuals
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
    let principal = ProfileId::from(auth.0.profile().id);
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

/// Read a container's composition
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
    let principal = ProfileId::from(auth.0.profile().id);
    let context_id = resolve_context_ref(&state.pool, principal, &cref).await?;

    let target = parse_composition_target(q.container, q.group.as_deref())?;
    let container_types = parse_container_types(q.container_types.as_deref());
    let depth = q.depth.unwrap_or(1);

    // A container drill seeds with the single container id; a bucket drill resolves its member
    // ids first. The bucket's container-walk uses `container_depth` — the depth the panorama
    // walked — not the drill `depth`, so the seed set is exactly the bucket the tray showed.
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
                    depth: q.container_depth.unwrap_or(CONTAINER_WALK_DEPTH),
                },
            )
            .await?
        }
    };

    context_graph_service::context_composition(&state.pool, principal, &seeds, depth)
        .await
        .map(Json)
}
