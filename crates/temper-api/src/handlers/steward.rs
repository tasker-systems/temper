//! `/api/steward` — the team-self-cognition steward's ingest trigger (T4a).
//!
//! `delta` (GET) is a service-direct read: the access gate (`anchor_readable_by_profile`) lives in
//! `steward_service::ingest_delta`, whose deny→NotFound surfaces as a uniform 404. `advance` (POST)
//! dispatches ONE operations command through the `Backend` trait; auth-before-write
//! (`cogmap_authorable_by_profile`) lives inside `DbBackend::advance_steward_watermark`, so the
//! handler just dispatches and lets the backend return 403/404.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::steward_service;
use temper_services::state::AppState;

use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::steward::{
    AdvanceWatermarkAck, AdvanceWatermarkRequest, DispatchTickRequest, DispatchTickResponse,
    DriftSweepRow, IngestDelta,
};
use temper_workflow::operations::{AdvanceStewardWatermark, Backend, StewardDispatchTick, Surface};

/// Query params for the delta read. `threshold` is optional (omit → the service default).
#[derive(Debug, Deserialize)]
pub struct DeltaQuery {
    pub threshold: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/steward/{cogmap}/delta",
    tag = "Steward",
    params(
        ("cogmap" = Uuid, Path, description = "Team-self-cognition cogmap id"),
        ("threshold" = Option<i64>, Query, description = "Ingest threshold to gate on (default applies when omitted)"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The ingest delta since the watermark", body = IngestDelta),
        (status = 404, description = "Cogmap not found, or not readable by the caller (uniform — no existence oracle)"),
    )
)]
pub async fn delta(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap): Path<Uuid>,
    Query(q): Query<DeltaQuery>,
) -> ApiResult<Json<IngestDelta>> {
    let delta = steward_service::ingest_delta(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        CogmapId::from(cogmap),
        q.threshold,
    )
    .await?;
    Ok(Json(delta))
}

#[utoipa::path(
    post,
    path = "/api/steward/{cogmap}/watermark",
    tag = "Steward",
    params(("cogmap" = Uuid, Path, description = "Team-self-cognition cogmap id")),
    security(("bearer_auth" = [])),
    request_body = AdvanceWatermarkRequest,
    responses(
        (status = 200, description = "Watermark advanced", body = AdvanceWatermarkAck),
        (status = 403, description = "Caller cannot author (write) this cogmap"),
        (status = 404, description = "Cogmap or event not found (uniform — no existence oracle)"),
    )
)]
pub async fn advance(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap): Path<Uuid>,
    Json(req): Json<AdvanceWatermarkRequest>,
) -> ApiResult<Json<AdvanceWatermarkAck>> {
    // Auth-before-write lives inside DbBackend::advance_steward_watermark — just dispatch.
    let cmd = AdvanceStewardWatermark {
        cogmap: CogmapId::from(cogmap),
        event_id: req.event_id,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .advance_steward_watermark(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(AdvanceWatermarkAck {
        cogmap_id: cogmap,
        watermark: out.value,
    }))
}

#[utoipa::path(
    get,
    path = "/api/steward/sweep",
    tag = "Steward",
    params(("threshold" = Option<i64>, Query, description = "Ingest threshold (default applies when omitted)")),
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Drifted team-joined cogmaps, most-drifted-first", body = Vec<DriftSweepRow>))
)]
pub async fn sweep(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<DeltaQuery>,
) -> ApiResult<Json<Vec<DriftSweepRow>>> {
    let rows =
        steward_service::drift_sweep(&state.pool, ProfileId::from(auth.0.profile.id), q.threshold)
            .await?;
    Ok(Json(rows))
}

#[utoipa::path(
    get,
    path = "/api/steward/candidates",
    tag = "Steward",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Readable team-joined cogmap ids", body = Vec<Uuid>))
)]
pub async fn candidates(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<Uuid>>> {
    let ids =
        steward_service::candidate_cogmaps(&state.pool, ProfileId::from(auth.0.profile.id)).await?;
    Ok(Json(ids))
}

#[utoipa::path(
    post,
    path = "/api/steward/dispatch",
    tag = "Steward",
    security(("bearer_auth" = [])),
    request_body = DispatchTickRequest,
    responses((status = 200, description = "Jobs claimed for fan-out", body = DispatchTickResponse))
)]
pub async fn dispatch(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<DispatchTickRequest>,
) -> ApiResult<Json<DispatchTickResponse>> {
    let cmd = StewardDispatchTick {
        threshold: req.threshold,
        cap: req.cap,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .steward_dispatch_tick(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(DispatchTickResponse { claimed: out.value }))
}
