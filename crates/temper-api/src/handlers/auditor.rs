//! `/api/auditor` — the citation auditor's dispatch tick (Set 5, Task 13).
//!
//! CONFORM to `handlers::steward`, deliberately and closely: `sweep` (GET) is a service-direct read
//! whose principal scoping lives in SQL (`audit_drift_sweep` routes through
//! `steward_candidate_cogmaps` and `resources_visible_to`), and `dispatch` (POST) dispatches ONE
//! operations command through the `Backend` trait. No persistence here.
//!
//! The auditor authenticates as its OWN registered machine principal, never the steward's — one
//! credential is one `emitter_entity_id`, and a shared client would leave the ledger unable to tell
//! an audit from the citation it audits (spec §5.2; `docs/auth/machine-token-contract.md` §C). That
//! is an operator/provisioning fact, not something this handler can enforce; what the handler does
//! guarantee is that everything downstream is scoped to `auth.0.profile().id`, so whichever
//! principal calls, it only ever sweeps and enqueues over what that principal can already read.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::auditor_service;
use temper_services::state::AppState;

use temper_core::types::auditor::{
    AuditSweepRow, AuditorDispatchTickRequest, AuditorDispatchTickResponse, AuditorJobCompleteAck,
};
use temper_core::types::ids::{CogmapId, CorrelationId, ProfileId};
use temper_workflow::operations::{AuditorDispatchTick, Backend, CompleteAuditorJob};

/// Query params for the sweep read. `cap` is optional (omit → the service default).
#[derive(Debug, Deserialize)]
pub struct SweepQuery {
    pub cap: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/auditor/sweep",
    tag = "Auditor",
    params(("cap" = Option<i64>, Query, description = "Max findings to return (default applies when omitted)")),
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Cogmap-homed findings with incomplete audit coverage, most-uncovered-first", body = Vec<AuditSweepRow>))
)]
pub async fn sweep(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<SweepQuery>,
) -> ApiResult<Json<Vec<AuditSweepRow>>> {
    let rows =
        auditor_service::drift_sweep(&state.pool, ProfileId::from(auth.0.profile().id), q.cap)
            .await?;
    Ok(Json(rows))
}

#[utoipa::path(
    post,
    path = "/api/auditor/dispatch",
    tag = "Auditor",
    security(("bearer_auth" = [])),
    request_body = AuditorDispatchTickRequest,
    responses((status = 200, description = "Citation-audit jobs claimed for fan-out, each carrying its cogmap's finding list", body = AuditorDispatchTickResponse))
)]
pub async fn dispatch(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    headers: HeaderMap,
    Json(req): Json<AuditorDispatchTickRequest>,
) -> ApiResult<Json<AuditorDispatchTickResponse>> {
    // Correlation trace, the same shape the steward's tick uses (design
    // 2026-07-06-steward-dispatch-correlation-id): the auditor cron mints a per-tick id and sends it
    // as `x-auditor-correlation-id`. Its OWN header name, not the steward's — the two crons run on
    // separate cadences under separate principals, and sharing a header key would make the two apps'
    // logs indistinguishable at exactly the moment someone is trying to tell them apart. Log it —
    // plus Vercel's own inbound `x-vercel-id` — on entry, before any DB work, so a tick that dies
    // here still leaves the id somewhere.
    let raw_correlation = headers
        .get("x-auditor-correlation-id")
        .and_then(|v| v.to_str().ok());
    let correlation_id = raw_correlation.unwrap_or("<none>");
    let vercel_id = headers
        .get("x-vercel-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");
    tracing::info!(correlation_id, vercel_id, "auditor dispatch received");

    // Parse leniently into the typed correlator that gets stamped on the claimed jobs. Correlation is
    // a correlation aid, never authorization — an absent or malformed header must self-root, never
    // 400. A malformed one is worth a warn: it means a caller believes it is tracing and is not.
    let correlation = raw_correlation.and_then(|raw| match Uuid::parse_str(raw) {
        Ok(id) => Some(CorrelationId::from(id)),
        Err(_) => {
            tracing::warn!(
                correlation_id,
                "x-auditor-correlation-id is not a UUID; this tick's jobs will self-root"
            );
            None
        }
    });

    let cmd = AuditorDispatchTick {
        cap: req.cap,
        correlation,
        origin: surface,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile().id));
    let out = backend
        .auditor_dispatch_tick(cmd)
        .await
        .map_err(ApiError::from)?;
    // Echo the correlation the server actually stamped — so the cron can assert its id survived
    // parsing rather than assuming it did.
    Ok(Json(AuditorDispatchTickResponse {
        claimed: out.value,
        correlation_id: correlation.map(|c| c.uuid()),
    }))
}

#[utoipa::path(
    post,
    path = "/api/auditor/{cogmap}/complete",
    tag = "Auditor",
    params(("cogmap" = Uuid, Path, description = "The cognitive map whose active citation-audit job is done")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The active citation-audit job was completed (or none was active)", body = AuditorJobCompleteAck),
        (status = 404, description = "Cogmap not found, or not readable by the caller (uniform — no existence oracle)"),
    )
)]
pub async fn complete(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Path(cogmap): Path<Uuid>,
) -> ApiResult<Json<AuditorJobCompleteAck>> {
    // Auth-before-write lives inside DbBackend::complete_auditor_job — just dispatch. There is no
    // request body: the completion carries no outcome. What the session actually decided lives in
    // the append-only audit trail it wrote and in its invocation close, not in a queue transition.
    let cmd = CompleteAuditorJob {
        cogmap: CogmapId::from(cogmap),
        origin: surface,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile().id));
    let out = backend
        .complete_auditor_job(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(AuditorJobCompleteAck {
        cogmap_id: cogmap,
        job_id: out.value,
    }))
}
