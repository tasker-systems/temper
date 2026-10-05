//! The block history scrub's HTTP doors (resource erasure spec 2026-09-28, build order 2e; D11):
//! the operator's execute door and the read-only survey beside it. They conform to the resource
//! erasure doors in [`crate::handlers::resource_erasure`], and that module's posture holds here.
//!
//! **A non-admin is rejected at the wire.** Each door mints the sealed `&SystemAdmin` proof
//! through `require_erasure_operator` before it dispatches. A caller the gate declines gets a
//! 404 (never 403) and one telemetry line, and the ledger gains nothing. The gate answers before
//! any lookup and before the service checks the block list, so every resource id and every
//! well-formed list gets the same 404 (axum's `Json` still rejects a malformed or unknown-field
//! body first; that names only the caller's own input).
//!
//! **No caller-supplied request reference.** The service mints the act's reference and both
//! execute answers return it. Both request bodies are `deny_unknown_fields`, so a body that
//! carries `request_reference` (or any other unknown field) is refused with axum's 422 instead
//! of being silently ignored.
//!
//! Both documented under the `Admin` tag (`routes/admin.rs`).

use axum::extract::State;
use axum::Json;

use temper_core::types::erasure::{
    BlockHistoryScrubExecuteResponse, BlockHistoryScrubRequestBody, BlockHistoryScrubSurvey,
};
use temper_core::types::ids::ResourceId;
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::block_history_scrub_service::{
    self, BlockHistoryScrubOutcome, BlockHistoryScrubRequest,
};
use temper_services::state::AppState;

use crate::handlers::erasure::require_erasure_operator;
use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;

/// `POST /api/admin/resources/block-history-scrub` — the operator's execute door.
#[utoipa::path(
    post,
    operation_id = "admin_scrub_block_history",
    summary = "Scrub the history of resource blocks",
    description = "Empties the history of the named blocks of a resource that is not erased: every revision but the current one and every non-current chunk of a live block, and every revision and chunk of a folded block. An in-flight ingest is cancelled and recorded. The server mints the request reference. The answer is either a completion or a recorded refusal (`status`). The list is checked against the resource's blocks first, whatever the resource's state, so a recorded refusal names only real blocks of the resource. Requires a system admin. Any other caller gets 404, decided before any lookup, so a refusal reveals nothing about the resource.",
    path = "/api/admin/resources/block-history-scrub",
    tag = "Admin",
    request_body = BlockHistoryScrubRequestBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The act completed, or was refused and the refusal recorded (`status` says which)", body = BlockHistoryScrubExecuteResponse),
        (status = 400, description = "`blocks` is empty, names a block twice, or names an id that is not a block of the resource, whatever the resource's state (checked before any refusal; nothing was scrubbed or recorded)", body = ErrorBody),
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
    Json(body): Json<BlockHistoryScrubRequestBody>,
) -> ApiResult<Json<BlockHistoryScrubExecuteResponse>> {
    let admin = require_erasure_operator(&state, &auth, "block_history_scrub.execute").await?;
    let outcome = block_history_scrub_service::execute_block_history_scrub(
        &state.pool,
        &admin,
        BlockHistoryScrubRequest {
            resource: ResourceId::from(body.resource),
            blocks: &body.blocks,
            surface,
        },
    )
    .await?;

    match outcome {
        BlockHistoryScrubOutcome::Refused(r) => {
            Ok(Json(BlockHistoryScrubExecuteResponse::Refused {
                request_reference: r.request_reference,
                event_id: r.event_id,
                reason: r.reason,
                detail: r.detail.map(|d| d.as_str().to_string()),
                blocks: body.blocks,
            }))
        }
        BlockHistoryScrubOutcome::Completed(c) => {
            Ok(Json(BlockHistoryScrubExecuteResponse::Completed {
                request_reference: c.request_reference,
                event_id: c.event_id,
                targets: c.targets,
                cancelled_ingest: c.cancelled_ingest,
            }))
        }
    }
}

/// `POST /api/admin/resources/block-history-scrub/survey` — the read-only survey. The gate
/// answers a caller who is not a system admin with the same 404 as execute and records nothing.
/// The service's survey types serialize as-is, so the door mirrors nothing field by field.
#[utoipa::path(
    post,
    operation_id = "admin_survey_block_history_scrub",
    summary = "Survey a block history scrub",
    description = "Reports what the block history scrub would do for the named blocks of a resource, without recording or changing anything: per block, whether it is folded and how many revisions and chunks it would empty, and whether an in-flight ingest would be cancelled. For a charter or an erased resource it reports the refusal the act would record (`refusal`, `detail`) and no plan, once the list names only blocks of the resource. The warning that a block's current revision still carries a sensitivity finding arrives with the sensitivity sweep (build order 3c); this survey reports counts only. Requires a system admin. Any other caller gets 404, decided before any lookup.",
    path = "/api/admin/resources/block-history-scrub/survey",
    tag = "Admin",
    request_body = BlockHistoryScrubRequestBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "What the act would do (`plan`), or the refusal it would record (`refusal`); nothing is recorded or changed", body = BlockHistoryScrubSurvey),
        (status = 400, description = "`blocks` is empty, names a block twice, or names an id that is not a block of the resource, whatever the resource's state (checked before any refusal is reported)", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin, answered before any lookup; or, for an admin, the resource does not exist", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape, e.g. a missing or unknown field (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn survey(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<BlockHistoryScrubRequestBody>,
) -> ApiResult<Json<BlockHistoryScrubSurvey>> {
    let admin = require_erasure_operator(&state, &auth, "block_history_scrub.survey").await?;
    let survey = block_history_scrub_service::survey_block_history_scrub(
        &state.pool,
        &admin,
        ResourceId::from(body.resource),
        &body.blocks,
    )
    .await?;
    Ok(Json(survey))
}
