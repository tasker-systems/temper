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
//! principal calls, it only ever sweeps, enqueues and CLAIMS over what that principal can already
//! read. "…and claims" is the fix wave's addition: the shipped claim filtered on
//! `(persona, dispatch_type, status)` alone, so this tick enqueued scoped work and then claimed
//! anyone's, handing the caller other tenants' `cogmap_id`s and finding-id lists.
//!
//! Authorization for `dispatch` and `complete` lives in `temper-services` (`authz/audit_gate.rs` and
//! the `DbBackend` commands), never here — these handlers hold no predicate, per
//! `.github/scripts/audit-handler-authz-drift.sh`. `sweep` deliberately keeps NO machine gate: it is
//! a read that returns only the caller's own readable findings, and it is the operator's debug view
//! of what the tick would do.
//!
//! # `GET /api/auditor/sweep` has no in-repo consumer, and it is KEPT — deliberately
//!
//! The review flagged it as over-build: nothing calls it (`schedules/auditor.ts` posts to
//! `/dispatch` only), yet it is a public, OpenAPI-contracted, SDK-generating route. Kept, for three
//! reasons that were weighed against deleting it:
//!
//! 1. **It is the only non-mutating way to ask what the tick would do.** `/dispatch` reaps,
//!    enqueues and *claims* — running it to see the queue takes the jobs `in_progress` under the
//!    caller. An operator debugging "why did nothing get audited this hour" has no other read, and
//!    the alternative (`psql` against the target) is not available to the agent that would ask.
//! 2. **Its sibling `GET /api/steward/sweep` exists for the same reason and has no consumer
//!    either.** Deleting one of a matched pair leaves the next reader deriving why the auditor
//!    surface is missing a read the steward has — asymmetry costs more than the route does.
//! 3. **It is not an enumeration oracle and it is not free-running.** `audit_drift_sweep` gates on
//!    `steward_candidate_cogmaps(caller)` *and* a per-finding `resources_visible_to(caller)`, and
//!    `cap` is validated here and clamped in `clamp_auditor_cap`. What remains is the scan cost the
//!    review names (the `scored` CTE prices every candidate before `p_limit` applies), which is a
//!    property of the sweep itself and applies equally to `/dispatch`.
//!
//! The cost of keeping it is a route in `openapi.json` and both SDKs. If a future pass finds the
//! scan cost matters, narrow the sweep — do not delete the read and leave `/dispatch` as the only
//! way to look.

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

/// Refuse a non-positive `cap` as the caller fault it is.
///
/// The service clamps into `[1, MAX_AUDITOR_DISPATCH_CAP]` regardless — that is the hard bound, and
/// it is what keeps every non-HTTP caller safe — but silently turning `cap: 0` into "one finding" is
/// answering a question the caller did not ask. Same two-layer shape as the audit value's range
/// guard: a 400 at the surface for the caller's benefit, a bound underneath for everyone else's.
/// A cap ABOVE the ceiling is deliberately NOT a 400: "as much as you can" is a coherent request and
/// the clamp answers it honestly.
fn validate_cap(cap: Option<i64>) -> ApiResult<()> {
    match cap {
        Some(c) if c < 1 => Err(ApiError::BadRequest(format!(
            "cap must be at least 1 (got {c})"
        ))),
        _ => Ok(()),
    }
}

/// Survey audit coverage across findings
#[utoipa::path(
    get,
    path = "/api/auditor/sweep",
    tag = "Auditor",
    // Explicit, because utoipa otherwise derives the operationId from the fn name and this
    // module's `sweep`/`dispatch` collide with `handlers::steward`'s identically-named fns. A
    // duplicate operationId is not cosmetic: it collides in the generated Ruby gem and TS client.
    operation_id = "auditor_sweep",
    params(("cap" = Option<i64>, Query, description = "Max findings to return (default applies when omitted)")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Cogmap-homed findings with incomplete audit coverage, most-uncovered-first", body = Vec<AuditSweepRow>),
        (status = 400, description = "cap below 1"),
    )
)]
pub async fn sweep(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<SweepQuery>,
) -> ApiResult<Json<Vec<AuditSweepRow>>> {
    validate_cap(q.cap)?;
    let rows =
        auditor_service::drift_sweep(&state.pool, ProfileId::from(auth.0.profile().id), q.cap)
            .await?;
    Ok(Json(rows))
}

/// Claim a batch of auditor jobs
#[utoipa::path(
    post,
    path = "/api/auditor/dispatch",
    tag = "Auditor",
    operation_id = "auditor_dispatch",
    security(("bearer_auth" = [])),
    request_body = AuditorDispatchTickRequest,
    responses(
        (status = 200, description = "Citation-audit jobs claimed for fan-out, each carrying its cogmap's finding list", body = AuditorDispatchTickResponse),
        (status = 400, description = "cap below 1"),
        (status = 403, description = "The caller is not a registered, unrevoked machine principal"),
    )
)]
pub async fn dispatch(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    headers: HeaderMap,
    Json(req): Json<AuditorDispatchTickRequest>,
) -> ApiResult<Json<AuditorDispatchTickResponse>> {
    validate_cap(req.cap)?;
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

/// Complete a claimed auditor job
#[utoipa::path(
    post,
    path = "/api/auditor/{cogmap}/complete",
    tag = "Auditor",
    operation_id = "complete_auditor_job",
    params(("cogmap" = Uuid, Path, description = "The cognitive map whose active citation-audit job is done")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The caller's in-flight citation-audit job was completed (job_id null when nothing of the caller's was in flight)", body = AuditorJobCompleteAck),
        (status = 404, description = "Cogmap not found, not readable by the caller, or the caller is not a registered machine principal (uniform — no existence oracle)"),
    )
)]
pub async fn complete(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Path(cogmap): Path<Uuid>,
) -> ApiResult<Json<AuditorJobCompleteAck>> {
    // Auth-before-write lives inside `DbBackend::complete_auditor_job` (`AuditorJobAuthority`, spec
    // §6.5) — just dispatch. There is no request body: the completion carries no outcome. What the
    // session actually decided lives in the append-only audit trail it wrote and in its invocation
    // close, not in a queue transition.
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
