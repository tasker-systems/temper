//! `POST /api/resources/reblock` — the corpus re-blocking wire surface.
//!
//! One bounded, resumable re-blocking step per call, dispatched to the existing `Backend` command
//! (`Backend::reblock_resources`): the handler authenticates, constructs the backend with the
//! caller's own identity, and dispatches. There is no admin gate here on purpose — the backend
//! is the shared seam, so the deployment-wide `all` scope is `is_system_admin`-gated inside
//! `DbBackend::reblock_resources` (a 403 surfacing through this route) while the resource and
//! context scopes enumerate through the caller's own visibility. This differs from the
//! operator-only `/api/embed/admin/reembed` trigger, which is admin-enclosed (the system-admin
//! group, `routes/admin.rs`): re-blocking is a per-row-gated verb whose contract is the receipt,
//! so it rides the general gated group.

use axum::extract::State;
use axum::Json;

use crate::middleware::auth::AnyPrincipal;
use crate::middleware::surface::RequestSurface;
use temper_core::types::reblock::{ReblockReceipt, ReblockRequest, DEFAULT_REBLOCK_LIMIT};
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;
use temper_workflow::operations::{Backend, ReblockResources};

/// Run one bounded, resumable re-blocking step
#[utoipa::path(
    post,
    operation_id = "reblock_resources",
    path = "/api/resources/reblock",
    tag = "Reblocking",
    request_body = ReblockRequest,
    security(("bearer_auth" = [])),
    responses(
        (
            status = 200,
            description = "The receipt for this step: one outcome row per candidate, per-class counts, the batch correlation id, and the continuation cursor to resume with. An invisible or absent context is never a 404: context scope enumerates no candidates and answers 200 with an empty `outcomes` array",
            body = ReblockReceipt,
        ),
        (
            status = 400,
            description = "The request violates the bounded-invocation contract (e.g. a non-positive limit)",
            body = ErrorBody,
        ),
        (status = 401, description = "Missing or invalid credentials", body = ErrorBody),
        (
            status = 403,
            description = "The deployment-wide `all` scope was requested by a caller who is not a system administrator (the resource and context scopes ride ordinary visibility instead and never refuse on reach alone)",
            body = ErrorBody,
        ),
        (
            status = 404,
            description = "The addressed resource does not exist or is not visible to the caller (resource scope only — context scope never answers 404; see the 200 description)",
            body = ErrorBody,
        ),
        (
            status = 410,
            description = "The addressed resource was erased (code RESOURCE_ERASED; resource scope only); answered only to a caller who held standing on it, everyone else gets 404. A candidate erased under a running batch is a `denied` row inside the 200, never a 410",
            body = ErrorBody,
        ),
    )
)]
pub async fn reblock(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    RequestSurface(surface): RequestSurface,
    Json(req): Json<ReblockRequest>,
) -> ApiResult<Json<ReblockReceipt>> {
    // Auth before anything else happens in the backend; the per-row gating lives there too —
    // this handler only dispatches, exactly like its Backend-dispatching siblings.
    let cmd = ReblockResources {
        scope: req.scope,
        dry_run: req.dry_run,
        limit: req.limit.unwrap_or(DEFAULT_REBLOCK_LIMIT),
        after_id: req.after_id,
        origin: surface,
    };
    let backend = DbBackend::with_proof(state.pool.clone(), &auth.0);
    let out = backend
        .reblock_resources(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}
