//! `PUT /api/cognitive-maps/{id}` — idempotent cognitive-map content reconcile, gated by regime in the
//! backend (`DbBackend::authorize_reconcile`): a system admin for L0 and gating-team maps, authorship
//! for every other map.
//!
//! The request body is a PRE-EMBEDDED desired-state manifest (the operator CLI embeds client-side). The
//! handler dispatches ONE operations command through the `Backend` trait, whose reconcile owns the
//! write gate (Auth before writes: the map's regime, decided once) — it never calls services or
//! `sqlx::query!` directly for the write.
//!
//! Also exposes `GET /api/cognitive-maps/{id}/shape` — the service-direct surface-tier region read.

use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::{AnyPrincipal, AuthUser};
use crate::middleware::surface::RequestSurface;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::{access_service, cogmap_service, materialize_service};
use temper_services::state::AppState;

use temper_core::types::cognitive_maps::{
    AnchorShape, BindTeamOutcome, BindTeamRequest, CogmapAnalyticsRow, CogmapDetail,
    CogmapGrantBody, CogmapRegionMetricsRow, CogmapRevokeBody, CogmapRow, GrantCapabilityRequest,
    GrantOutcome, RevokeCapabilityRequest, RevokeOutcome, UnbindTeamOutcome,
};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::materialize::{MaterializeAck, MaterializeDelta, MaterializeRequest};
use temper_core::types::query_params::{MaterializeDeltaQuery, ShapeQuery};
use temper_core::types::reconcile::{
    CreateCogmapOutcome, CreateCogmapRequest, ReconcileCogmapRequest, ReconcileOutcome,
};
use temper_workflow::operations::{
    Backend, CreateCognitiveMap, MaterializeOnThreshold, ReconcileCognitiveMap,
};

/// Reconcile a cognitive map's contents
#[utoipa::path(
    put,
    path = "/api/cognitive-maps/{id}",
    tag = "Cognitive Maps",
    params(
        ("id" = Uuid, Path, description = "Cognitive map ID"),
        temper_core::types::authorship::ActInput
    ),
    security(("bearer_auth" = [])),
    request_body = ReconcileCogmapRequest,
    responses(
        (status = 200, description = "Reconcile applied", body = ReconcileOutcome),
        (status = 400, description = "The manifest or the act is invalid: it fails the pre-flight (an edge naming an unknown target), names an unknown edge kind or polarity, or carries malformed authorship fields", body = temper_services::error::ErrorBody),
        (status = 401, description = "Unauthorized", body = temper_services::error::ErrorBody),
        (status = 403, description = "Caller does not author this map, or is not a system admin for an L0 or root-team map", body = temper_services::error::ErrorBody),
        (status = 409, description = "A reconcile is already in progress on this map"),
    )
)]
pub async fn reconcile(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    RequestSurface(surface): RequestSurface,
    Path(cogmap_id): Path<Uuid>,
    Query(act_in): Query<temper_core::types::authorship::ActInput>,
    Json(request): Json<ReconcileCogmapRequest>,
) -> ApiResult<Json<ReconcileOutcome>> {
    // Auth before writes lives in the backend, which decides the map's regime once (admin-only
    // map → system admin; any other → authorship) — see `DbBackend::reconcile_cognitive_map`.

    // The manifest body stays pure; authorship rides query params (reconcile uses only
    // `act.authorship` — its invocation is server-minted). Reassembled here, validated once.
    let act = act_in.into_act_context().map_err(ApiError::from)?;

    let cmd = ReconcileCognitiveMap {
        cogmap_id: CogmapId::from(cogmap_id),
        request,
        act,
        origin: surface,
    };
    let backend = DbBackend::with_proof(state.pool.clone(), &auth.0);
    let out = backend
        .reconcile_cognitive_map(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}

/// Create a cognitive map and its telos charter
#[utoipa::path(
    post,
    path = "/api/cognitive-maps",
    tag = "Cognitive Maps",
    security(("bearer_auth" = [])),
    request_body = CreateCogmapRequest,
    responses(
        (status = 200, description = "Genesis applied (or idempotent no-op)", body = CreateCogmapOutcome),
        (status = 403, description = "Caller has no approved principal standing"),
        (status = 409, description = "A concurrent genesis conflicted; retry"),
    )
)]
pub async fn genesis(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    RequestSurface(surface): RequestSurface,
    Json(request): Json<CreateCogmapRequest>,
) -> ApiResult<Json<CreateCogmapOutcome>> {
    // Genesis is open to any authenticated profile. The reserved-id guard and the creator-grant live
    // in the backend command (`create_cognitive_map`): a caller-supplied id is honored only for a
    // system-admin, and the creator is granted read+write+grant on the new map.
    let cmd = CreateCognitiveMap {
        request,
        origin: surface,
    };
    let backend = DbBackend::with_proof(state.pool.clone(), &auth.0);
    let out = backend
        .create_cognitive_map(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}

/// List cognitive maps you can see
#[utoipa::path(
    get,
    operation_id = "list_cognitive_maps",
    path = "/api/cognitive-maps",
    tag = "Cognitive Maps",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The caller's visible cognitive maps (identity + charter statement)", body = Vec<CogmapRow>),
        (status = 401, description = "Unauthorized", body = temper_services::error::ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AnyPrincipal,
) -> ApiResult<Json<Vec<CogmapRow>>> {
    // No entry gate: `list_visible` is self-scoped through `cogmap_visible_maps` — it returns exactly
    // the maps the caller may see (deny → empty list).
    cogmap_service::list_visible(&state.pool, ProfileId::from(auth.0.profile().id))
        .await
        .map(Json)
}

/// Get one cognitive map
#[utoipa::path(
    get,
    operation_id = "get_cognitive_map",
    path = "/api/cognitive-maps/{id}",
    tag = "Cognitive Maps",
    params(("id" = Uuid, Path, description = "Cognitive map ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The map's identity, charter, and foundational resources", body = CogmapDetail),
        (status = 404, description = "Map not found or not readable"),
    )
)]
pub async fn show(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(cogmap_id): Path<Uuid>,
) -> ApiResult<Json<CogmapDetail>> {
    // Map-read gated inside `show_visible`: an unreadable map is 404, never a partial leak.
    cogmap_service::show_visible(&state.pool, ProfileId::from(auth.0.profile().id), cogmap_id)
        .await
        .map(Json)
}

/// Read a cognitive map's shape
#[utoipa::path(
    get,
    path = "/api/cognitive-maps/{id}/shape",
    tag = "Cognitive Maps",
    params(
        ("id" = Uuid, Path, description = "Cognitive map ID"),
        ("lens" = Option<Uuid>, Query, description = "Optional lens filter; omit for all lenses"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Materialized regions (surface tier), wrapped in an envelope whose `emptiness` names why an empty answer is empty", body = AnchorShape),
        (status = 401, description = "Unauthorized", body = temper_services::error::ErrorBody),
    )
)]
pub async fn shape(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(cogmap_id): Path<Uuid>,
    Query(q): Query<ShapeQuery>,
) -> ApiResult<Json<AnchorShape>> {
    temper_services::backend::substrate_read::anchor_shape_select(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        HomeAnchor::Cogmap(CogmapId::from(cogmap_id)),
        q.lens,
    )
    .await
    .map(Json)
}

/// Read formation drift since last materialize
#[utoipa::path(
    get,
    path = "/api/cognitive-maps/{id}/materialize-delta",
    tag = "Cognitive Maps",
    params(
        ("id" = Uuid, Path, description = "Cognitive map ID"),
        ("threshold" = Option<i64>, Query, description = "Materialize threshold to gate on (default applies when omitted)"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The materialize delta since the last materialize", body = MaterializeDelta),
        (status = 404, description = "Cogmap not found, or not readable by the caller (uniform — no existence oracle)"),
    )
)]
pub async fn materialize_delta(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(cogmap_id): Path<Uuid>,
    Query(q): Query<MaterializeDeltaQuery>,
) -> ApiResult<Json<MaterializeDelta>> {
    let delta = materialize_service::materialize_delta(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        HomeAnchor::Cogmap(CogmapId::from(cogmap_id)),
        q.threshold,
    )
    .await?;
    Ok(Json(delta))
}

/// Materialize a cognitive map's regions
#[utoipa::path(
    post,
    path = "/api/cognitive-maps/{id}/materialize",
    tag = "Cognitive Maps",
    params(("id" = Uuid, Path, description = "Cognitive map ID")),
    security(("bearer_auth" = [])),
    request_body = MaterializeRequest,
    responses(
        (status = 200, description = "Materialize ran (over threshold) or was a no-op (below)", body = MaterializeAck),
        (status = 403, description = "Caller cannot author (write) this cogmap"),
        (status = 404, description = "Cogmap not found (uniform — no existence oracle)"),
    )
)]
pub async fn materialize(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    RequestSurface(surface): RequestSurface,
    Path(cogmap_id): Path<Uuid>,
    Json(req): Json<MaterializeRequest>,
) -> ApiResult<Json<MaterializeAck>> {
    // Auth-before-write + the threshold gate live inside DbBackend::materialize_on_threshold — just dispatch.
    let cmd = MaterializeOnThreshold {
        anchor: HomeAnchor::Cogmap(CogmapId::from(cogmap_id)),
        threshold: req.threshold,
        origin: surface,
    };
    let backend = DbBackend::with_proof(state.pool.clone(), &auth.0);
    let out = backend
        .materialize_on_threshold(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}

/// Read per-region metrics for a map
#[utoipa::path(
    get,
    path = "/api/cognitive-maps/{id}/region-metrics",
    tag = "Cognitive Maps",
    params(
        ("id" = Uuid, Path, description = "Cognitive map ID"),
        ("lens" = Option<Uuid>, Query, description = "Optional lens filter; omit for all lenses"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Per-region analytics-tier scalar metrics", body = Vec<CogmapRegionMetricsRow>),
        (status = 401, description = "Unauthorized", body = temper_services::error::ErrorBody),
    )
)]
pub async fn region_metrics(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(cogmap_id): Path<Uuid>,
    Query(q): Query<ShapeQuery>,
) -> ApiResult<Json<Vec<CogmapRegionMetricsRow>>> {
    temper_services::backend::substrate_read::anchor_region_metrics_select(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        HomeAnchor::Cogmap(CogmapId::from(cogmap_id)),
        q.lens,
    )
    .await
    .map(Json)
}

/// Read cognitive-map analytics
#[utoipa::path(
    get,
    path = "/api/cognitive-maps/{id}/analytics",
    tag = "Cognitive Maps",
    params(("id" = Uuid, Path, description = "Cognitive map ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Map-level analytics (telos, staleness, regulation)", body = CogmapAnalyticsRow),
        (status = 404, description = "Map not found or not readable"),
        (status = 401, description = "Unauthorized", body = temper_services::error::ErrorBody),
    )
)]
pub async fn analytics(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(cogmap_id): Path<Uuid>,
) -> ApiResult<Json<CogmapAnalyticsRow>> {
    temper_services::backend::substrate_read::cogmap_analytics_select(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        cogmap_id,
    )
    .await?
    .map(Json)
    .ok_or_else(|| ApiError::NotFound("cognitive map not found or not readable".to_string()))
}

/// Bind a team to a cognitive map
#[utoipa::path(
    post,
    path = "/api/cognitive-maps/{id}/teams",
    tag = "Cognitive Maps",
    params(("id" = Uuid, Path, description = "Cognitive map ID")),
    security(("bearer_auth" = [])),
    request_body = BindTeamRequest,
    responses(
        (status = 200, description = "Team bound (or idempotent no-op)", body = BindTeamOutcome),
        (status = 403, description = "Caller is not a system admin and does not both administer the map and manage the team"),
    )
)]
pub async fn bind_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Json(body): Json<BindTeamRequest>,
) -> ApiResult<Json<BindTeamOutcome>> {
    // Auth before writes lives in the service (`TwoSidedAuthority`), so every surface —
    // MCP included, which binds to these same routes — is gated identically.
    let outcome = cogmap_service::bind_team(&state.pool, &auth.0, cogmap_id, &body).await?;
    Ok(Json(outcome))
}

/// Unbind a team from a cognitive map
#[utoipa::path(
    delete,
    path = "/api/cognitive-maps/{id}/teams/{team_id}",
    tag = "Cognitive Maps",
    params(
        ("id" = Uuid, Path, description = "Cognitive map ID"),
        ("team_id" = Uuid, Path, description = "Team ID to unbind"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Team unbound (or no-op)", body = UnbindTeamOutcome),
        (status = 403, description = "Caller is not a system admin and does not both administer the map and manage the team"),
    )
)]
pub async fn unbind_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((cogmap_id, team_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<UnbindTeamOutcome>> {
    let outcome = cogmap_service::unbind_team(&state.pool, &auth.0, cogmap_id, team_id).await?;
    Ok(Json(outcome))
}

/// Grant access to a cognitive map
#[utoipa::path(
    post,
    operation_id = "grant_cogmap_access",
    path = "/api/cognitive-maps/{id}/grants",
    tag = "Cognitive Maps",
    params(("id" = Uuid, Path, description = "Cognitive map ID")),
    security(("bearer_auth" = [])),
    request_body = CogmapGrantBody,
    responses(
        (status = 200, description = "Grant minted (or updated)", body = GrantOutcome),
        (status = 403, description = "Caller may not administer grants on this map"),
    )
)]
pub async fn grant(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Json(body): Json<CogmapGrantBody>,
) -> ApiResult<Json<GrantOutcome>> {
    // Auth before writes lives in the service (`is_system_admin OR can(...,'grant',...)`), shared with
    // the MCP surface. The subject is the path cogmap; widen the body into the polymorphic request.
    let req = GrantCapabilityRequest {
        subject_table: "kb_cogmaps".to_string(),
        subject_id: cogmap_id,
        principal_table: body.principal_table,
        principal_id: body.principal_id,
        can_read: body.can_read,
        can_write: body.can_write,
        can_delete: body.can_delete,
        can_grant: body.can_grant,
    };
    let outcome = access_service::grant_capability(&state.pool, &auth.0, &req).await?;
    Ok(Json(outcome))
}

/// Revoke access to a cognitive map
#[utoipa::path(
    delete,
    operation_id = "revoke_cogmap_access",
    path = "/api/cognitive-maps/{id}/grants",
    tag = "Cognitive Maps",
    params(("id" = Uuid, Path, description = "Cognitive map ID")),
    security(("bearer_auth" = [])),
    request_body = CogmapRevokeBody,
    responses(
        (status = 200, description = "Grant revoked (or no-op)", body = RevokeOutcome),
        (status = 403, description = "Caller may not administer grants on this map"),
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Json(body): Json<CogmapRevokeBody>,
) -> ApiResult<Json<RevokeOutcome>> {
    let req = RevokeCapabilityRequest {
        subject_table: "kb_cogmaps".to_string(),
        subject_id: cogmap_id,
        principal_table: body.principal_table,
        principal_id: body.principal_id,
    };
    let outcome = access_service::revoke_capability(&state.pool, &auth.0, &req).await?;
    Ok(Json(outcome))
}
