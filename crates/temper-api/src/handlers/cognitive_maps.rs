//! `PUT /api/cognitive-maps/{id}` — admin-gated, idempotent cognitive-map content reconcile.
//!
//! The request body is a PRE-EMBEDDED desired-state manifest (the operator CLI embeds client-side). The
//! handler enforces the root-team-cogmap write gate (Auth before writes), then dispatches ONE operations
//! command through the `Backend` trait — it never calls services or `sqlx::query!` directly for the write.
//!
//! Also exposes `GET /api/cognitive-maps/{id}/shape` — the service-direct surface-tier region read.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::backend::DbBackend;
use crate::middleware::auth::AuthUser;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::{access_service, cogmap_service};
use temper_services::state::AppState;

use temper_core::types::cognitive_maps::{
    BindTeamOutcome, BindTeamRequest, CogmapAnalyticsRow, CogmapRegionMetricsRow, CogmapRegionRow,
    UnbindTeamOutcome,
};
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::reconcile::{
    CreateCogmapOutcome, CreateCogmapRequest, ReconcileCogmapRequest, ReconcileOutcome,
};
use temper_workflow::operations::{Backend, CreateCognitiveMap, ReconcileCognitiveMap, Surface};

/// Query params for the shape read. `lens` is optional (omit → all lenses).
#[derive(Debug, Deserialize)]
pub struct ShapeQuery {
    pub lens: Option<Uuid>,
}

#[utoipa::path(
    put,
    path = "/api/cognitive-maps/{id}",
    tag = "Cognitive Maps",
    params(("id" = Uuid, Path, description = "Cognitive map ID")),
    security(("bearer_auth" = [])),
    request_body = ReconcileCogmapRequest,
    responses(
        (status = 200, description = "Reconcile applied", body = ReconcileOutcome),
        (status = 403, description = "Caller is not a system admin for this root-team map"),
        (status = 409, description = "A reconcile is already in progress on this map"),
    )
)]
pub async fn reconcile(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Query(act_in): Query<temper_core::types::authorship::ActInput>,
    Json(request): Json<ReconcileCogmapRequest>,
) -> ApiResult<Json<ReconcileOutcome>> {
    // Auth before writes (Global Constraints): the root-team-cogmap write gate.
    access_service::require_cogmap_write_admin(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        CogmapId::from(cogmap_id),
    )
    .await?;

    // The manifest body stays pure; authorship rides query params (reconcile uses only
    // `act.authorship` — its invocation is server-minted). Reassembled here, validated once.
    let act = act_in.into_act_context().map_err(ApiError::from)?;

    let cmd = ReconcileCognitiveMap {
        cogmap_id: CogmapId::from(cogmap_id),
        request,
        act,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .reconcile_cognitive_map(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}

#[utoipa::path(
    post,
    path = "/api/cognitive-maps",
    tag = "Cognitive Maps",
    security(("bearer_auth" = [])),
    request_body = CreateCogmapRequest,
    responses(
        (status = 200, description = "Genesis applied (or idempotent no-op)", body = CreateCogmapOutcome),
        (status = 403, description = "Caller is not a system admin"),
        (status = 409, description = "A concurrent genesis conflicted; retry"),
    )
)]
pub async fn genesis(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(request): Json<CreateCogmapRequest>,
) -> ApiResult<Json<CreateCogmapOutcome>> {
    // Auth before writes (Global Constraints): genesis is system-admin-only. A brand-new cogmap id is
    // neither the reserved L0 kernel nor yet root-team-bound, so `require_cogmap_write_admin` would
    // FAIL-OPEN here — the genesis gate is plain `is_system_admin`, checked at the TOP before any write.
    let profile_id = ProfileId::from(auth.0.profile.id);
    if !access_service::is_system_admin(&state.pool, profile_id).await? {
        return Err(ApiError::Forbidden);
    }

    let cmd = CreateCognitiveMap {
        request,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), profile_id);
    let out = backend
        .create_cognitive_map(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}

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
        (status = 200, description = "Materialized regions (surface tier)", body = Vec<CogmapRegionRow>),
        (status = 401, description = "Unauthorized", body = temper_services::error::ErrorBody),
    )
)]
pub async fn shape(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Query(q): Query<ShapeQuery>,
) -> ApiResult<Json<Vec<CogmapRegionRow>>> {
    crate::backend::substrate_read::cogmap_shape_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
        q.lens,
    )
    .await
    .map(Json)
}

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
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Query(q): Query<ShapeQuery>,
) -> ApiResult<Json<Vec<CogmapRegionMetricsRow>>> {
    crate::backend::substrate_read::cogmap_region_metrics_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
        q.lens,
    )
    .await
    .map(Json)
}

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
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
) -> ApiResult<Json<CogmapAnalyticsRow>> {
    crate::backend::substrate_read::cogmap_analytics_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
    )
    .await?
    .map(Json)
    .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post,
    path = "/api/cognitive-maps/{id}/teams",
    tag = "Cognitive Maps",
    params(("id" = Uuid, Path, description = "Cognitive map ID")),
    security(("bearer_auth" = [])),
    request_body = BindTeamRequest,
    responses(
        (status = 200, description = "Team bound (or idempotent no-op)", body = BindTeamOutcome),
        (status = 403, description = "Caller is not a system admin"),
    )
)]
pub async fn bind_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Json(body): Json<BindTeamRequest>,
) -> ApiResult<Json<BindTeamOutcome>> {
    // Auth before writes lives in the service (`is_system_admin`), so the MCP
    // surface — which calls the service directly — is gated identically.
    let outcome = cogmap_service::bind_team(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
        &body,
    )
    .await?;
    Ok(Json(outcome))
}

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
        (status = 403, description = "Caller is not a system admin"),
    )
)]
pub async fn unbind_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((cogmap_id, team_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<UnbindTeamOutcome>> {
    let outcome = cogmap_service::unbind_team(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
        team_id,
    )
    .await?;
    Ok(Json(outcome))
}
