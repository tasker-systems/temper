//! The field scrub's HTTP doors (field-grain scrub spec 2026-10-09, S1, S4, S5): the operator's
//! execute door, the read-only survey beside it, and the family listing an operator picks a handle
//! from. They conform to the block history scrub's doors in
//! [`crate::handlers::block_history_scrub`], and that module's posture holds here.
//!
//! **A non-admin is rejected at the wire.** Each door mints the sealed `&SystemAdmin` proof
//! through `require_erasure_operator` before it dispatches. A caller the gate declines gets a
//! 404 (never 403) and one telemetry line, and the ledger gains nothing. The gate answers before
//! any lookup and before the service checks the request, so every resource id and every
//! well-formed request gets the same 404 (axum's `Json` still rejects a malformed or unknown-field
//! body first; that names only the caller's own input).
//!
//! **No text crosses.** A request names a field by kind and a family by an event id; no door
//! answers or logs a key or a value. **No caller-supplied request reference.** The service mints
//! the act's reference and both execute answers return it. Every request body is
//! `deny_unknown_fields`, so a body that carries `request_reference` (or any other unknown field)
//! is refused with axum's 422 instead of being silently ignored.
//!
//! All three documented under the `Admin` tag (`routes/admin.rs`).

use axum::extract::State;
use axum::Json;

use temper_core::types::erasure::{
    FieldScrubExecuteResponse, FieldScrubFamilies, FieldScrubFamiliesRequest,
    FieldScrubRequestBody, FieldScrubSurvey,
};
use temper_core::types::ids::ResourceId;
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::field_scrub_service::{self, FieldScrubOutcome, FieldScrubRequest};
use temper_services::state::AppState;
use temper_workflow::operations::Surface;

use crate::handlers::erasure::require_erasure_operator;
use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;

/// The service's request from the wire body.
fn request(body: &FieldScrubRequestBody, surface: Surface) -> FieldScrubRequest {
    FieldScrubRequest {
        resource: ResourceId::from(body.resource),
        field: body.field,
        family: body.family,
        clear: body.clear,
        surface,
    }
}

/// `POST /api/admin/resources/field-scrub` — the operator's execute door.
#[utoipa::path(
    post,
    operation_id = "admin_scrub_resource_field",
    summary = "Scrub a resource field's history",
    description = "Redacts every prior value of a resource's title, origin URI, one property family (named by its handle from the family listing) or every property family, from the ledger and the projection, while the resource survives. By default today's value is kept; with `clear`, the act first clears it (a placeholder title or origin URI, the placeholder type for `doc_type`, or an unset of the family) and then redacts every value the field held. `properties` cannot be cleared. The server mints the request reference. The answer is either a completion or a recorded refusal (`status`). The request and the handle are checked first, whatever the resource's state, so a recorded refusal names only a real family of the resource. No key text or value appears in the request, the answer or the record. Requires a system admin. Any other caller gets 404, decided before any lookup, so a refusal reveals nothing about the resource.",
    path = "/api/admin/resources/field-scrub",
    tag = "Admin",
    request_body = FieldScrubRequestBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The act completed, or was refused and the refusal recorded (`status` says which)", body = FieldScrubExecuteResponse),
        (status = 400, description = "A handle with a field that takes none, `property` without a handle, `properties` with `clear`, a handle that is not a property family of the resource (checked before any refusal), in keep mode nothing prior to scrub, or with `clear` a field already cleared (the placeholder title or origin URI, the placeholder type, or a family with no live row); nothing was scrubbed or recorded", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin, answered before any lookup; or, for an admin, the resource does not exist", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape, e.g. a missing or unknown field or an unknown field kind: a caller-supplied `request_reference` is refused, not ignored (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn execute(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Json(body): Json<FieldScrubRequestBody>,
) -> ApiResult<Json<FieldScrubExecuteResponse>> {
    let admin = require_erasure_operator(&state, &auth, "field_scrub.execute").await?;
    let outcome =
        field_scrub_service::execute_field_scrub(&state.pool, &admin, request(&body, surface))
            .await?;

    match outcome {
        FieldScrubOutcome::Refused(r) => Ok(Json(FieldScrubExecuteResponse::Refused {
            request_reference: r.request_reference,
            event_id: r.event_id,
            reason: r.reason,
            detail: r.detail.map(|d| d.as_str().to_string()),
            field: r.field,
        })),
        FieldScrubOutcome::Completed(c) => Ok(Json(FieldScrubExecuteResponse::Completed {
            request_reference: c.request_reference,
            event_id: c.event_id,
            field: c.field,
            cleared: c.cleared,
            redacted_fields: c.redacted_fields,
        })),
    }
}

/// `POST /api/admin/resources/field-scrub/survey` — the read-only survey. The gate answers a
/// caller who is not a system admin with the same 404 as execute and records nothing. The
/// service's survey types serialize as-is.
#[utoipa::path(
    post,
    operation_id = "admin_survey_resource_field_scrub",
    summary = "Survey a field scrub",
    description = "Reports what the field scrub would do, without recording or changing anything: the ledger paths it would redact, the paths it keeps (today's value) or cannot reach and why, the folded property rows it would rewrite, and the events `clear` would append, beside the family listing with the sensitivity sweep's flags. For a charter, an erased resource, a sentinel collision or a title or origin URI whose latest event disagrees with the projection it reports the refusal the act would record (`refusal`, `detail`) and no plan, once the request and its handle are well formed. In keep mode a plan with no `redacted_fields` means nothing is prior, which the act answers 400; with `clear`, a field already cleared is answered 400 here as the act answers it. Requires a system admin. Any other caller gets 404, decided before any lookup.",
    path = "/api/admin/resources/field-scrub/survey",
    tag = "Admin",
    request_body = FieldScrubRequestBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "What the act would do (`plan`), or the refusal it would record (`refusal`); nothing is recorded or changed", body = FieldScrubSurvey),
        (status = 400, description = "A handle with a field that takes none, `property` without a handle, `properties` with `clear`, or a handle that is not a property family of the resource, whatever the resource's state (checked before any refusal is reported); or, with `clear`, a field already cleared, as the act answers it", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin, answered before any lookup; or, for an admin, the resource does not exist", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape, e.g. a missing or unknown field or an unknown field kind (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn survey(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Json(body): Json<FieldScrubRequestBody>,
) -> ApiResult<Json<FieldScrubSurvey>> {
    let admin = require_erasure_operator(&state, &auth, "field_scrub.survey").await?;
    let survey =
        field_scrub_service::survey_field_scrub(&state.pool, &admin, request(&body, surface))
            .await?;
    Ok(Json(survey))
}

/// `POST /api/admin/resources/field-scrub/families` — the family listing. The gate answers a
/// caller who is not a system admin with the same 404 as the other doors and records nothing.
#[utoipa::path(
    post,
    operation_id = "admin_list_resource_field_families",
    summary = "List a resource's scrubbable fields",
    description = "Lists what a field scrub can name on a resource, without recording or changing anything: the title, the origin URI and each resource-owned property family by its handle (the id of the first event still carrying its key text), with its event count, whether it is live or unset, when and by which profile it was first seen, the JSON type of its latest value, and the sensitivity sweep's flags. No key text and no value. An erased resource or a charter, which the act refuses before it reads a family, lists nothing. Requires a system admin. Any other caller gets 404, decided before any lookup.",
    path = "/api/admin/resources/field-scrub/families",
    tag = "Admin",
    request_body = FieldScrubFamiliesRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The resource's scrubbable fields; nothing is recorded or changed", body = FieldScrubFamilies),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin, answered before any lookup; or, for an admin, the resource does not exist", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape, e.g. a missing or unknown field (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn families(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<FieldScrubFamiliesRequest>,
) -> ApiResult<Json<FieldScrubFamilies>> {
    let admin = require_erasure_operator(&state, &auth, "field_scrub.families").await?;
    let families = field_scrub_service::list_field_scrub_families(
        &state.pool,
        &admin,
        ResourceId::from(body.resource),
    )
    .await?;
    Ok(Json(families))
}
