//! The erasure act's HTTP surfaces (spec 2026-08-31, task 01a0577c Beat 4; the survey door is
//! task 01a09628 item 2): the operator's execute door, the read-only survey beside it, and the
//! byte-delete fence's cron tick.
//!
//! **The doors reject a non-admin at the wire.** Every erasure service function takes the sealed
//! `&SystemAdmin` proof, so each door mints it with `require_system_admin` before it dispatches
//! (`require_erasure_operator`, shared with [`crate::handlers::resource_erasure`]), the
//! `admin_directory` shape. A caller the gate declines is answered **404, never 403**, before any
//! lookup, so every subject id gets the same body and the refused caller learns nothing about the
//! SUBJECT (not whether it exists, not whether it was erased). The doors themselves are
//! discoverable, and the 404 does not claim to hide them. The only record of that attempt is one
//! `tracing` line: no ledger event of any kind. A survey attempt was never recorded either (ruled 2026-09-12), so both doors now answer a
//! non-admin alike. Both are documented under the `Admin` tag (`routes/admin.rs`): the contract
//! states the 404, because the 404 protects the subject, not the door.
//!
//! The 404 covers WELL-FORMED requests: axum's `Json` extractor rejects a malformed body before
//! the handler runs, the scope `admin_directory` states for its own gate. Such a rejection names
//! only the caller's own malformed input.
//!
//! **The drain is the fence's only driver.** Same internal-cron posture as
//! `/api/embed/dispatch`: bearer-gated by the shared `EMBED_DISPATCH_SECRET` (no new secret —
//! the fail-closed-variable hazard `require_dispatch_secret` exists to avoid), GET+POST on one
//! handler, excluded from the contract entirely.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Serialize;

use temper_core::types::erasure::{
    BlobStrikeView, ErasureExecuteRequest, ErasureExecuteResponse, ErasureSurveyRequest,
    ErasureSurveyResponse,
};
use temper_core::types::ids::ProfileId;
use temper_services::auth::SystemAdmin;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::services::erasure_fence_service::{self, DrainSummary};
use temper_services::services::erasure_service;
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;

/// The erasure doors' gate: the sealed `&SystemAdmin` proof, or a does-not-exist 404.
///
/// `require_system_admin`'s `Forbidden` becomes `NotFound("not found")`, the same body for every
/// id, because the gate answers before anything is looked up. The attempt is recorded by this one
/// `warn` line and by nothing else: no ledger event. Warn, the level the API already logs every
/// `Forbidden` at (`ApiError`'s response logging): the 404 it is rendered as logs at debug, so
/// without this line a non-admin probing an erasure door would leave no trace an operator sees at
/// the default filter. Any other gate failure (the governance read itself) propagates unchanged.
pub(crate) async fn require_erasure_operator(
    state: &AppState,
    auth: &AuthUser,
    door: &'static str,
) -> ApiResult<SystemAdmin> {
    match temper_services::auth::require_system_admin(&state.pool, &auth.0).await {
        Err(ApiError::Forbidden) => {
            tracing::warn!(
                profile_id = %auth.0.profile().id,
                door,
                "erasure door refused a caller who is not a system admin; answered 404, \
                 nothing recorded on the ledger"
            );
            Err(ApiError::NotFound("not found".to_string()))
        }
        gate => gate,
    }
}

/// `POST /api/admin/erasure` — the operator's execute door.
///
/// The gate runs here, before dispatch (`require_erasure_operator`); the service takes the proof
/// and attributes the act to `admin.actor()`.
///
/// The request reference tolerates retries: a retried POST with the SAME reference re-executes
/// as a no-op completion (the subject is already tombstoned, so the act completes with
/// `already_erased: true`). Correlation is INDEXED, never unique
/// (20260624000001_canonical_schema.sql:491) — the reference pairs the act's events, it does
/// not deduplicate the door.
#[utoipa::path(
    post,
    operation_id = "admin_erase_principal",
    summary = "Erase a principal",
    description = "Executes the erasure act for a principal, identified by pseudonym and an opaque request reference. Repeating it with the same reference completes as a no-op with `already_erased: true`. Requires a system admin. Any other caller gets 404, decided before any lookup, so a refusal reveals nothing about the subject.",
    path = "/api/admin/erasure",
    tag = "Admin",
    request_body = ErasureExecuteRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The erasure completed (`already_erased` on a repeat)", body = ErasureExecuteResponse),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin, answered before any lookup; or, for an admin, the subject does not exist", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape, e.g. a missing or unknown field (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn execute(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Json(body): Json<ErasureExecuteRequest>,
) -> ApiResult<Json<ErasureExecuteResponse>> {
    let admin = require_erasure_operator(&state, &auth, "erasure.execute").await?;
    let c = erasure_service::execute_erasure(
        &state.pool,
        &admin,
        ProfileId::from(body.subject),
        body.request_reference,
        surface,
    )
    .await?;

    Ok(Json(ErasureExecuteResponse::Completed {
        event_id: c.event_id,
        already_erased: c.already_erased,
        redacted_hashes: c.redacted_hashes,
        targets: c.targets,
        blob_strikes: c
            .blob_strikes
            .into_iter()
            .map(|s| BlobStrikeView {
                blob_id: s.blob_id,
                released: s.released,
            })
            .collect(),
        resource_erasures: c.resource_erasures,
        estate_stragglers: c.estate_stragglers,
    }))
}

/// `POST /api/admin/erasure/survey` — the read-only survey beside the execute door (task
/// 01a09628 item 2).
///
/// Gated here like execute (`require_erasure_operator`): a caller who is not a system admin
/// gets the same 404 and no event. The survey is witnessed read-only: no events, no projection
/// change — it previews, it never prepares.
#[utoipa::path(
    post,
    operation_id = "admin_survey_principal_erasure",
    summary = "Survey a principal erasure",
    description = "Reports what the erasure act would do for a principal, without recording or changing anything. Requires a system admin. Any other caller gets 404, decided before any lookup.",
    path = "/api/admin/erasure/survey",
    tag = "Admin",
    request_body = ErasureSurveyRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "What the erasure act would do; nothing is recorded or changed", body = ErasureSurveyResponse),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin, answered before any lookup; or, for an admin, the subject does not exist", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape, e.g. a missing or unknown field (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn survey(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ErasureSurveyRequest>,
) -> ApiResult<Json<ErasureSurveyResponse>> {
    let admin = require_erasure_operator(&state, &auth, "erasure.survey").await?;
    let prediction =
        erasure_service::survey_erasure(&state.pool, &admin, ProfileId::from(body.subject)).await?;

    Ok(Json(ErasureSurveyResponse {
        subject: prediction.subject.uuid(),
        already_erased: prediction.already_erased,
        redacted_hashes: prediction.redacted_hashes,
        targets: prediction.targets,
        blob_strikes: prediction
            .blob_strikes
            .into_iter()
            .map(|s| BlobStrikeView {
                blob_id: s.blob_id,
                released: s.released,
            })
            .collect(),
        estate: prediction.estate,
        resources: prediction.resources,
    }))
}

/// The fence drain's response: the tick's tallies plus whether a provider is configured.
#[derive(Debug, Serialize)]
pub struct FenceDrainResponse {
    #[serde(flatten)]
    pub drain: DrainSummary,
    pub store_configured: bool,
}

/// `GET|POST /api/erasure/drain` — one byte-delete fence tick.
///
/// Seeding is STORE-INDEPENDENT: the pending deletes are derived from the ledger, never from a
/// provider, so a tick with no provider config still seeds every erasure's released strikes and
/// still ages them into the alertable state ([`erasure_fence_service::drain_without_store`]) —
/// the substrate contract's stranding posture ("retry plus age alerting") has no
/// store-configured precondition, and a deployment that HAD a provider and lost its
/// configuration must go LOUD about its stranded deletes, never quiet. Only the provider CALLS
/// (claim → delete → resolve) require the store.
pub async fn drain(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<FenceDrainResponse>> {
    crate::handlers::embed::require_dispatch_secret(&state, &headers, "erasure delete drain")?;

    match state.blob_store.as_deref() {
        Some(store) => {
            let summary = erasure_fence_service::drain(&state.pool, store).await?;
            tracing::info!(
                seeded = summary.seeded,
                unparseable_verdicts = summary.unparseable_verdicts,
                claimed = summary.claimed,
                deleted = summary.deleted,
                skipped = summary.skipped_reoccupied,
                failed = summary.failed,
                "erasure fence drain pass complete"
            );
            Ok(Json(FenceDrainResponse {
                drain: summary,
                store_configured: true,
            }))
        }
        None => {
            let summary = erasure_fence_service::drain_without_store(&state.pool).await?;
            tracing::info!(
                seeded = summary.seeded,
                unparseable_verdicts = summary.unparseable_verdicts,
                store_configured = false,
                "erasure fence seed-only pass complete (no provider configured)"
            );
            Ok(Json(FenceDrainResponse {
                drain: summary,
                store_configured: false,
            }))
        }
    }
}
