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

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Serialize;

use temper_core::types::erasure::{
    BlobStrikeView, ErasureExecuteRequest, ErasureExecuteResponse, ErasureSurveyRequest,
    ErasureSurveyResponse, RetentionCategory, RetentionPolicies, RetentionPolicySet,
};
use temper_core::types::ids::ProfileId;
use temper_services::auth::SystemAdmin;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::link_provider::{self, LinkProvider};
use temper_services::services::erasure_fence_service::{self, DrainSummary};
use temper_services::services::erasure_slack_revocation_service::{
    self, SlackRevocationDrain, SlackRevokeConfig,
};
use temper_services::services::slack_disconnect_service::IdpRevokeTarget;
use temper_services::services::{erasure_retention_service, erasure_service};
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
    let provider = slack_link_provider(&state);
    let c = erasure_service::execute_erasure(
        &state.pool,
        &admin,
        ProfileId::from(body.subject),
        body.request_reference,
        surface,
        slack_revoke_config(&state, provider.as_ref()).as_ref(),
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
        slack_revocation: c.slack_revocation,
    }))
}

/// The instance's Slack revoke endpoint and its client, or `None` where Slack linking is not
/// configured. The provider owns the strings [`slack_revoke_config`] borrows.
fn slack_link_provider(state: &AppState) -> Option<LinkProvider> {
    let cfg = state.config.slack_link.as_ref()?;
    Some(link_provider::derive(&state.config.auth, cfg))
}

/// What revoking a Slack grant needs: the vault key, and where and as whom to revoke. The same
/// endpoint and client a disconnect revokes at (`handlers::slack_disconnect`).
fn slack_revoke_config<'a>(
    state: &'a AppState,
    provider: Option<&'a LinkProvider>,
) -> Option<SlackRevokeConfig<'a>> {
    let cfg = state.config.slack_link.as_ref()?;
    let provider = provider?;
    Some(SlackRevokeConfig {
        key: &cfg.vault_key,
        target: IdpRevokeTarget {
            mode: state.config.auth.mode,
            revoke_url: &provider.revoke_url,
            client_id: &cfg.client_id,
        },
    })
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
    /// The Slack revocations erasures queued (`erasure_slack_revocation_service`), retried on the
    /// same tick.
    pub slack_revocations: SlackRevocationDrain,
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

    let (drain, store_configured) = match state.blob_store.as_deref() {
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
            (summary, true)
        }
        None => {
            let summary = erasure_fence_service::drain_without_store(&state.pool).await?;
            tracing::info!(
                seeded = summary.seeded,
                unparseable_verdicts = summary.unparseable_verdicts,
                store_configured = false,
                "erasure fence seed-only pass complete (no provider configured)"
            );
            (summary, false)
        }
    };

    // The Slack revocations erasures queued: a sibling pass on the same tick, after the fence. It
    // runs with or without Slack configured — without it, only the rows past their window are
    // emptied, so a deployment that lost its configuration still never keeps a token past it.
    let provider = slack_link_provider(&state);
    let slack_revocations = erasure_slack_revocation_service::drain(
        &state.pool,
        slack_revoke_config(&state, provider.as_ref()).as_ref(),
    )
    .await?;
    tracing::info!(
        attempted = slack_revocations.attempted,
        revoked = slack_revocations.revoked,
        retrying = slack_revocations.retrying,
        abandoned = slack_revocations.abandoned,
        "erasure Slack revocation pass complete"
    );

    Ok(Json(FenceDrainResponse {
        drain,
        store_configured,
        slack_revocations,
    }))
}

/// `GET /api/admin/erasure/retention` — the deployment's retention policy for person erasure.
///
/// Gated like the erasure doors (`require_erasure_operator`): a caller who is not a system admin
/// gets 404 and nothing is recorded.
#[utoipa::path(
    get,
    operation_id = "admin_list_erasure_retention",
    summary = "List the erasure retention policy",
    description = "Lists the categories a person erasure keeps, each with its policy's name, basis and period. Empty by default: the act then keeps nothing. Requires a system admin. Any other caller gets 404.",
    path = "/api/admin/erasure/retention",
    tag = "Admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The retention policy", body = RetentionPolicies),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin", body = ErrorBody),
    )
)]
pub async fn list_retention(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<RetentionPolicies>> {
    let admin = require_erasure_operator(&state, &auth, "erasure.retention.list").await?;
    Ok(Json(
        erasure_retention_service::list(&state.pool, &admin).await?,
    ))
}

/// `PUT /api/admin/erasure/retention` — set the policy for one category.
#[utoipa::path(
    put,
    operation_id = "admin_set_erasure_retention",
    summary = "Set an erasure retention policy",
    description = "Names a category a person erasure keeps, with the basis and the period, replacing the category's existing policy. Only free text and replay evidence can be kept; identifiers and credentials cannot. The act records each kept category and its policy. Nothing deletes kept data when the period ends. Requires a system admin. Any other caller gets 404.",
    path = "/api/admin/erasure/retention",
    tag = "Admin",
    request_body = RetentionPolicySet,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The retention policy after the change", body = RetentionPolicies),
        (status = 400, description = "A malformed name, an empty basis, or a period that is not a positive interval", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin", body = ErrorBody),
        (status = 409, description = "The name already names another category's policy", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn set_retention(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<RetentionPolicySet>,
) -> ApiResult<Json<RetentionPolicies>> {
    let admin = require_erasure_operator(&state, &auth, "erasure.retention.set").await?;
    Ok(Json(
        erasure_retention_service::set(&state.pool, &admin, &body).await?,
    ))
}

/// `DELETE /api/admin/erasure/retention/{category}` — stop keeping a category.
#[utoipa::path(
    delete,
    operation_id = "admin_remove_erasure_retention",
    summary = "Remove an erasure retention policy",
    description = "Stops keeping a category: a person erasure run afterwards removes it. Removing a category with no policy is a no-op. Requires a system admin. Any other caller gets 404.",
    path = "/api/admin/erasure/retention/{category}",
    tag = "Admin",
    params(("category" = RetentionCategory, Path, description = "The category to stop keeping")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The retention policy after the change", body = RetentionPolicies),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`), answered by the access gate before the admin check", body = ErrorBody),
        (status = 404, description = "Caller is not a system admin", body = ErrorBody),
    )
)]
pub async fn remove_retention(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(category): Path<RetentionCategory>,
) -> ApiResult<Json<RetentionPolicies>> {
    let admin = require_erasure_operator(&state, &auth, "erasure.retention.remove").await?;
    Ok(Json(
        erasure_retention_service::remove(&state.pool, &admin, category).await?,
    ))
}
