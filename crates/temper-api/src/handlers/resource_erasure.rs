//! The resource-erasure act's HTTP doors (resource erasure spec 2026-09-28, build order 2b): the
//! operator's execute door and the read-only survey beside it. They conform to the principal
//! doors in [`crate::handlers::erasure`] and their posture is documented there.
//!
//! **A non-admin is rejected at the wire, like the principal doors.** Each door mints the sealed
//! `&SystemAdmin` proof through `require_erasure_operator` before it
//! dispatches, and the service functions take that proof. A caller the gate declines gets a 404
//! (never 403) and one telemetry line, and the ledger gains nothing. The refused caller learns
//! nothing about the RESOURCE: the gate answers before any lookup and before the service checks
//! the blob list, so every id and every well-formed blob list gets the same 404, which says neither
//! that the resource exists nor that it was erased (axum's `Json` still rejects a malformed or
//! unknown-field body first; that names only the caller's own input). The doors themselves are
//! discoverable, and the 404 does not claim to hide them. No tenant axis exists: the gate is the
//! instance operator and nothing more (ruled 2026-09-30).
//!
//! **No caller-supplied request reference.** The service mints the act's reference and both
//! execute answers return it. The execute body is `deny_unknown_fields`, so a body that carries
//! `request_reference` is refused at the door (axum's `Json` answers a well-formed body with an
//! unknown field as 422) instead of being silently ignored.
//!
//! Both documented under the `Admin` tag (`routes/admin.rs`).

use axum::extract::State;
use axum::Json;

use temper_core::types::erasure::{
    BlobStrikeView, ResourceErasureExecuteRequest, ResourceErasureExecuteResponse,
    ResourceErasureSurvey, ResourceErasureSurveyRequest,
};
use temper_core::types::ids::{BlobId, ResourceId};
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::resource_erasure_service::{
    self, ResourceErasureOutcome, ResourceErasureRequest,
};
use temper_services::state::AppState;

use crate::handlers::erasure::require_erasure_operator;
use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;

/// `POST /api/admin/resources/erasure` — the operator's execute door.
#[utoipa::path(
    post,
    operation_id = "admin_erase_resource",
    summary = "Erase a resource",
    description = "Executes the erasure act for a resource, optionally striking related blobs named by the survey. The server mints the request reference. The answer is either a completion or a recorded refusal (`status`). Requires a system admin. Any other caller gets 404, decided before any lookup, so a refusal reveals nothing about the resource.",
    path = "/api/admin/resources/erasure",
    tag = "Admin",
    request_body = ResourceErasureExecuteRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The act completed, or was refused and the refusal recorded (`status` says which)", body = ResourceErasureExecuteResponse),
        (status = 400, description = "`also_strike_blobs` names a blob twice, or one the act refuses to strike (the act rolled back; nothing was struck)", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin, answered before any lookup; or, for an admin, the resource does not exist", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape, e.g. a missing or unknown field: a caller-supplied `request_reference` is refused, not ignored (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn execute(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Json(body): Json<ResourceErasureExecuteRequest>,
) -> ApiResult<Json<ResourceErasureExecuteResponse>> {
    let admin = require_erasure_operator(&state, &auth, "resource_erasure.execute").await?;
    let blobs: Vec<BlobId> = body
        .also_strike_blobs
        .unwrap_or_default()
        .into_iter()
        .map(BlobId::from)
        .collect();
    let outcome = resource_erasure_service::execute_resource_erasure(
        &state.pool,
        state.blob_store.as_deref(),
        &admin,
        ResourceErasureRequest {
            resource: ResourceId::from(body.resource),
            also_strike_blobs: &blobs,
            surface,
        },
    )
    .await?;

    match outcome {
        ResourceErasureOutcome::Refused(r) => Ok(Json(ResourceErasureExecuteResponse::Refused {
            request_reference: r.request_reference,
            event_id: r.event_id,
            reason: r.reason,
            detail: r.detail.map(|d| d.as_str().to_string()),
        })),
        ResourceErasureOutcome::Completed(c) => {
            Ok(Json(ResourceErasureExecuteResponse::Completed {
                request_reference: c.request_reference,
                event_id: c.event_id,
                folded_edges: c.folded_edges,
                targets: c.targets,
                remainder: c.remainder,
                ledger_remainder: c.ledger_remainder,
                blob_strikes: c
                    .blob_strikes
                    .into_iter()
                    .map(|s| BlobStrikeView {
                        blob_id: s.blob_id,
                        released: s.released,
                    })
                    .collect(),
            }))
        }
    }
}

/// `POST /api/admin/resources/erasure/survey` — the read-only survey. The gate answers a caller
/// who is not a system admin with the same 404 as execute and records nothing; an unknown id past
/// the gate is a 404 too. The service's survey types serialize as-is (`Serialize` derived on
/// them), so the door mirrors nothing field by field.
#[utoipa::path(
    post,
    operation_id = "admin_survey_resource_erasure",
    summary = "Survey a resource erasure",
    description = "Reports what the erasure act would do for a resource, without recording or changing anything. `plan` is absent when the resource was already erased. Requires a system admin. Any other caller gets 404, decided before any lookup.",
    path = "/api/admin/resources/erasure/survey",
    tag = "Admin",
    request_body = ResourceErasureSurveyRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "What the act would do (`plan` is absent when the resource was already erased); nothing is recorded or changed", body = ResourceErasureSurvey),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin, answered before any lookup; or, for an admin, the resource does not exist", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape, e.g. a missing or unknown field (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn survey(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ResourceErasureSurveyRequest>,
) -> ApiResult<Json<ResourceErasureSurvey>> {
    let admin = require_erasure_operator(&state, &auth, "resource_erasure.survey").await?;
    let survey = resource_erasure_service::survey_resource_erasure(
        &state.pool,
        &admin,
        ResourceId::from(body.resource),
    )
    .await?;
    Ok(Json(survey))
}
