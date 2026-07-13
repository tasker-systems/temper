use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;
use temper_core::types::cognitive_maps::{CogmapRegionMetricsRow, CogmapRegionRow};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::materialize::{MaterializeAck, MaterializeRequest};
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::context_service::{
    self, ContextCreateRequest, ContextRow, ContextRowWithCounts, ShareContextOutcome,
    ShareContextRequest, UnshareContextOutcome,
};
use temper_services::state::AppState;
use temper_workflow::operations::{Backend, MaterializeOnThreshold};

#[utoipa::path(
    get,
    operation_id = "list_contexts",
    path = "/api/contexts",
    tag = "Contexts",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of visible contexts with resource counts", body = Vec<ContextRowWithCounts>),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<ContextRowWithCounts>>> {
    context_service::list_visible(&state.pool, ProfileId::from(auth.0.profile.id))
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    operation_id = "create_context",
    path = "/api/contexts",
    tag = "Contexts",
    security(("bearer_auth" = [])),
    request_body = ContextCreateRequest,
    responses(
        (status = 201, description = "Context created", body = ContextRow),
        (status = 409, description = "Context name already exists"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ContextCreateRequest>,
) -> ApiResult<(StatusCode, Json<ContextRow>)> {
    let caller = ProfileId::from(auth.0.profile.id);
    let (owner_table, owner_id) =
        context_service::resolve_create_owner(&state.pool, caller, body.owner.as_ref()).await?;
    let row = context_service::create(&state.pool, &owner_table, owner_id, &body.name).await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[utoipa::path(
    get,
    operation_id = "get_context",
    path = "/api/contexts/{id}",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Context details", body = ContextRow),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
) -> ApiResult<Json<ContextRow>> {
    context_service::get_visible(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        ContextId::from(context_id),
    )
    .await
    .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/contexts/{id}/teams",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    request_body = ShareContextRequest,
    responses(
        (status = 200, description = "Context shared (or idempotent no-op)", body = ShareContextOutcome),
        (status = 403, description = "Caller may not share this context into this team"),
    )
)]
pub async fn share_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Json(body): Json<ShareContextRequest>,
) -> ApiResult<Json<ShareContextOutcome>> {
    let outcome = context_service::share(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        context_id,
        &body,
    )
    .await?;
    Ok(Json(outcome))
}

#[utoipa::path(
    delete,
    path = "/api/contexts/{id}/teams/{team_id}",
    tag = "Contexts",
    params(
        ("id" = Uuid, Path, description = "Context ID"),
        ("team_id" = Uuid, Path, description = "Team ID to unshare"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Context unshared (or no-op)", body = UnshareContextOutcome),
        (status = 403, description = "Caller may not unshare this context from this team"),
    )
)]
pub async fn unshare_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((context_id, team_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<UnshareContextOutcome>> {
    let outcome = context_service::unshare(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        context_id,
        team_id,
    )
    .await?;
    Ok(Json(outcome))
}

// ─────────────────────────────────────────────────────────────────────────────
// Context orientation reads (spec §3.7, T8) — the region-level view of a context.
//
// The peer of `/api/cognitive-maps/{id}/{shape,region-metrics,materialize}`, and deliberately the
// SAME wire types: a region row carries nothing cogmap-specific, so `CogmapRegionRow` describes a
// context's region exactly as well (the `cogmap_*` naming goes away at M3, not the shape).
//
// Every gate lives in the SQL (`anchor_readable_by_profile` → `context_readable_by_profile`), so a
// caller who cannot read the context gets an empty list rather than a 403 — no existence oracle.

/// Query params for the context shape / region-metrics reads.
#[derive(Debug, Deserialize)]
pub struct ContextShapeQuery {
    /// Optional lens filter; omit for all lenses.
    pub lens: Option<Uuid>,
}

#[utoipa::path(
    get,
    operation_id = "context_shape",
    path = "/api/contexts/{id}/shape",
    tag = "Contexts",
    params(
        ("id" = Uuid, Path, description = "Context ID"),
        ("lens" = Option<Uuid>, Query, description = "Optional lens filter; omit for all lenses"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The context's materialized regions (surface tier), most salient first", body = Vec<CogmapRegionRow>),
        (status = 401, description = "Unauthorized", body = temper_services::error::ErrorBody),
    )
)]
pub async fn shape(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Query(q): Query<ContextShapeQuery>,
) -> ApiResult<Json<Vec<CogmapRegionRow>>> {
    temper_services::backend::substrate_read::anchor_shape_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        HomeAnchor::Context(ContextId::from(context_id)),
        q.lens,
    )
    .await
    .map(Json)
}

#[utoipa::path(
    get,
    operation_id = "context_region_metrics",
    path = "/api/contexts/{id}/region-metrics",
    tag = "Contexts",
    params(
        ("id" = Uuid, Path, description = "Context ID"),
        ("lens" = Option<Uuid>, Query, description = "Optional lens filter; omit for all lenses"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Per-region analytics-tier scalar metrics for the context", body = Vec<CogmapRegionMetricsRow>),
        (status = 401, description = "Unauthorized", body = temper_services::error::ErrorBody),
    )
)]
pub async fn region_metrics(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Query(q): Query<ContextShapeQuery>,
) -> ApiResult<Json<Vec<CogmapRegionMetricsRow>>> {
    temper_services::backend::substrate_read::anchor_region_metrics_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        HomeAnchor::Context(ContextId::from(context_id)),
        q.lens,
    )
    .await
    .map(Json)
}

#[utoipa::path(
    post,
    operation_id = "context_materialize",
    path = "/api/contexts/{id}/materialize",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    request_body = MaterializeRequest,
    responses(
        (status = 200, description = "Materialize ran (over threshold) or was a no-op (below)", body = MaterializeAck),
        (status = 403, description = "Caller cannot author (write) this context"),
        (status = 404, description = "Context not found (uniform — no existence oracle)"),
    )
)]
pub async fn materialize(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Path(context_id): Path<Uuid>,
    Json(req): Json<MaterializeRequest>,
) -> ApiResult<Json<MaterializeAck>> {
    // Auth-before-write + the threshold gate live inside DbBackend::materialize_on_threshold, which is
    // anchor-generic — the context arm gates on `context_authorable_by_profile` and materializes under
    // `workflow-default`. Just dispatch.
    let cmd = MaterializeOnThreshold {
        anchor: HomeAnchor::Context(ContextId::from(context_id)),
        threshold: req.threshold,
        origin: surface,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .materialize_on_threshold(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}
