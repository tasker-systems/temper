use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;
use temper_core::types::cognitive_maps::{AnchorShape, CogmapRegionMetricsRow, CogmapStaleness};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::materialize::{MaterializeAck, MaterializeDelta, MaterializeRequest};
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::context_service::{
    self, ContextCreateRequest, ContextRow, ContextRowWithCounts, ReassignContextOutcome,
    ReassignContextRequest, RenameContextOutcome, RenameContextRequest, ShareContextOutcome,
    ShareContextRequest, UnshareContextOutcome,
};
use temper_services::services::materialize_service;
use temper_services::state::AppState;
use temper_workflow::operations::{Backend, MaterializeOnThreshold};

/// List contexts you can see
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
    context_service::list_visible(&state.pool, ProfileId::from(auth.0.profile().id))
        .await
        .map(Json)
}

/// Create a context
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
    let caller = ProfileId::from(auth.0.profile().id);
    let (owner_table, owner_id) =
        context_service::resolve_create_owner(&state.pool, caller, body.owner.as_ref()).await?;
    let row =
        context_service::create(&state.pool, caller, &owner_table, owner_id, &body.name).await?;
    Ok((StatusCode::CREATED, Json(row)))
}

/// Get one context
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
        ProfileId::from(auth.0.profile().id),
        ContextId::from(context_id),
    )
    .await
    .map(Json)
}

/// Share a context with a team
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
        ProfileId::from(auth.0.profile().id),
        context_id,
        &body,
    )
    .await?;
    Ok(Json(outcome))
}

/// Stop sharing a context with a team
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
        ProfileId::from(auth.0.profile().id),
        context_id,
        team_id,
    )
    .await?;
    Ok(Json(outcome))
}

/// Reassign a context to another owner
#[utoipa::path(
    post,
    path = "/api/contexts/{id}/reassign",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    request_body = ReassignContextRequest,
    responses(
        (status = 200, description = "Context ownership transferred (or idempotent no-op)", body = ReassignContextOutcome),
        (status = 403, description = "Caller may not transfer this context to this team"),
        (status = 404, description = "Context or team not found"),
        (status = 409, description = "Target team already owns a context with this slug"),
    )
)]
pub async fn reassign(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Json(body): Json<ReassignContextRequest>,
) -> ApiResult<Json<ReassignContextOutcome>> {
    let outcome = context_service::reassign(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        context_id,
        body.to_team_id,
    )
    .await?;
    Ok(Json(outcome))
}

/// Rename a context
#[utoipa::path(
    post,
    path = "/api/contexts/{id}/rename",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    request_body = RenameContextRequest,
    responses(
        (status = 200, description = "Context renamed (or idempotent no-op when the canonical name already matched)", body = RenameContextOutcome),
        (status = 400, description = "The name derives an empty slug (empty, whitespace-only, or no sluggifiable characters)"),
        (status = 403, description = "Caller may read but not administer this context"),
        (status = 404, description = "Context not found (uniform — no existence oracle for a caller who cannot see it)"),
        (status = 409, description = "Another context under the same owner already holds the derived slug"),
    )
)]
pub async fn rename(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Json(body): Json<RenameContextRequest>,
) -> ApiResult<Json<RenameContextOutcome>> {
    let outcome = context_service::rename(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        context_id,
        &body.name,
    )
    .await?;
    Ok(Json(outcome))
}

// ─────────────────────────────────────────────────────────────────────────────
// Context orientation reads (spec §3.7, T8) — the region-level view of a context.
//
// The peer of the cognitive-map orientation reads —
// `/api/cognitive-maps/{id}/{shape,region-metrics,materialize,materialize-delta,analytics}` — and
// deliberately the SAME wire types: a region row carries nothing cogmap-specific, so
// `CogmapRegionRow` describes a context's region exactly as well (the `cogmap_*` naming goes away at
// M3, not the shape).
//
// Every gate lives in the SQL (`anchor_readable_by_profile` → `context_readable_by_profile`), so a
// caller who cannot read the context gets `emptiness: unreadable_or_absent` rather than a 403 — an
// arm that collapses "denied" and "does not exist" and discloses neither the population nor the
// clock, so it is still no existence oracle.

/// Query params for the context shape / region-metrics reads.
#[derive(Debug, Deserialize)]
pub struct ContextShapeQuery {
    /// Optional lens filter; omit for all lenses.
    pub lens: Option<Uuid>,
}

/// Read a context's shape
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
        (status = 200, description = "The context's materialized regions (surface tier), most salient first, wrapped in an envelope whose `emptiness` names why an empty answer is empty", body = AnchorShape),
        (status = 401, description = "Unauthorized", body = temper_services::error::ErrorBody),
    )
)]
pub async fn shape(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Query(q): Query<ContextShapeQuery>,
) -> ApiResult<Json<AnchorShape>> {
    temper_services::backend::substrate_read::anchor_shape_select(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        HomeAnchor::Context(ContextId::from(context_id)),
        q.lens,
    )
    .await
    .map(Json)
}

/// Read per-region metrics for a context
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
        ProfileId::from(auth.0.profile().id),
        HomeAnchor::Context(ContextId::from(context_id)),
        q.lens,
    )
    .await
    .map(Json)
}

/// Query params for the context materialize-delta read. `threshold` is optional (omit → the
/// service default).
#[derive(Debug, Deserialize)]
pub struct ContextMaterializeDeltaQuery {
    /// Materialize threshold to gate on; the service default applies when omitted.
    pub threshold: Option<i64>,
}

/// Read formation drift since a context's last materialize
///
/// The read peer of `POST /api/contexts/{id}/materialize`: T8 gave a context the ability to
/// materialize but not the ability to be asked when that last happened.
///
/// Deny is 404 here, not an empty envelope — the posture of `/shape` next door does NOT travel to
/// this route. Absent and unreadable are collapsed, so it is still no existence oracle.
#[utoipa::path(
    get,
    operation_id = "context_materialize_delta",
    path = "/api/contexts/{id}/materialize-delta",
    tag = "Contexts",
    params(
        ("id" = Uuid, Path, description = "Context ID"),
        ("threshold" = Option<i64>, Query, description = "Materialize threshold to gate on (default applies when omitted)"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The materialize delta since the context's last materialize", body = MaterializeDelta),
        (status = 404, description = "Context not found, or not readable by the caller (uniform — no existence oracle)"),
    )
)]
pub async fn materialize_delta(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Query(q): Query<ContextMaterializeDeltaQuery>,
) -> ApiResult<Json<MaterializeDelta>> {
    let delta = materialize_service::materialize_delta(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        HomeAnchor::Context(ContextId::from(context_id)),
        q.threshold,
    )
    .await?;
    Ok(Json(delta))
}

/// Materialize a context's regions
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
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile().id));
    let out = backend
        .materialize_on_threshold(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}

/// Read a context's analytics
///
/// The last asymmetric row of the anchor read surface: `shape`, `region-metrics`,
/// `materialize-delta` and `materialize` were already symmetric across the two anchor kinds and
/// `analytics` was cogmap-only.
///
/// **Three fields, not the five of `/api/cognitive-maps/{id}/analytics`.** A context has no charter
/// resource and no regulation set, so `telos_resource_id` and `regulation` would be null peer fields
/// reporting "nothing found" about two things that cannot exist. The shape difference is the answer,
/// not a gap in it.
///
/// Deny is 404 here, matching the cogmap peer and `materialize-delta` next door — NOT the 200-with-
/// `emptiness` posture of `/shape`. Absent and unreadable are collapsed (the SQL yields zero rows for
/// both), so it is still no existence oracle.
#[utoipa::path(
    get,
    operation_id = "context_analytics",
    path = "/api/contexts/{id}/analytics",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Context-level staleness: when the shape was last materialized, the latest readable touch, and whether the read is stale", body = CogmapStaleness),
        (status = 404, description = "Context not found, or not readable by the caller (uniform — no existence oracle)"),
        (status = 401, description = "Unauthorized", body = temper_services::error::ErrorBody),
    )
)]
pub async fn analytics(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
) -> ApiResult<Json<CogmapStaleness>> {
    temper_services::backend::substrate_read::context_analytics_select(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        context_id,
    )
    .await?
    .map(Json)
    .ok_or_else(|| ApiError::NotFound("context not found or not readable".to_string()))
}
