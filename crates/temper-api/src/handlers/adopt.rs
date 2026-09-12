//! `POST /api/resources/adopt` — the corpus-adoption wire surface.
//!
//! One bounded, resumable adoption step per call, dispatched to the existing `Backend` command
//! (`Backend::adopt_resources`): the handler authenticates, constructs the backend with the
//! caller's own identity, and dispatches. There is no admin gate here on purpose — the backend
//! is the shared seam, so the deployment-wide `all` scope is `is_system_admin`-gated inside
//! `DbBackend::adopt_resources` (a 403 surfacing through this route) while the resource and
//! context scopes enumerate through the caller's own visibility. This differs from the
//! operator-only `/api/embed/admin/reembed` trigger, which is admin-enclosed and deliberately
//! undocumented: adoption is a per-row-gated operator verb whose contract is the receipt, so it
//! is a documented route in the OpenAPI contract.

use axum::extract::State;
use axum::Json;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;
use temper_core::types::adoption::{AdoptReceipt, AdoptRequest, DEFAULT_ADOPT_LIMIT};
use temper_core::types::ids::ProfileId;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;
use temper_workflow::operations::{AdoptResources, Backend};

/// Run one corpus-adoption step
#[utoipa::path(
    post,
    operation_id = "adopt_resources",
    path = "/api/resources/adopt",
    tag = "Adoption",
    request_body = AdoptRequest,
    security(("bearer_auth" = [])),
    responses(
        (
            status = 200,
            description = "The receipt for this step: one outcome row per candidate, per-class counts, the batch correlation id, and the continuation cursor to resume with",
            body = AdoptReceipt,
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
            description = "The addressed resource or context does not exist or is not visible to the caller",
            body = ErrorBody,
        ),
    )
)]
pub async fn adopt(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Json(req): Json<AdoptRequest>,
) -> ApiResult<Json<AdoptReceipt>> {
    // Auth before anything else happens in the backend; the per-row gating lives there too —
    // this handler only dispatches, exactly like its Backend-dispatching siblings.
    let cmd = AdoptResources {
        scope: req.scope,
        dry_run: req.dry_run,
        limit: req.limit.unwrap_or(DEFAULT_ADOPT_LIMIT),
        after_id: req.after_id,
        origin: surface,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile().id));
    let out = backend.adopt_resources(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}
