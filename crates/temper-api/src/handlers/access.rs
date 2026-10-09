//! Handlers for the system access gate endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use temper_core::types::access_gate::{
    JoinRequest, JoinRequestWithProfile, PublicSystemSettings, QueueCount,
    ReconcileAutoJoinOutcome, ReviewRequestWithProfile, SystemSettings,
};
use temper_core::types::admin::{DemoteAdminRequest, PromoteAdminRequest, UpdateSettingsRequest};
use temper_core::types::ids::ProfileId;
use temper_core::types::team::TeamMemberRow;

use crate::middleware::auth::AuthUser;
use temper_core::types::access_gate::{
    CloseReviewBody, CreateRequestBody, CreateReviewBody, ReviewRequestBody, RevokePrincipalBody,
};
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::access_service;
use temper_services::state::AppState;

// ---------------------------------------------------------------------------
// Request body types
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Public endpoints (auth_only router)
// ---------------------------------------------------------------------------

/// Request to join the gating team
#[utoipa::path(
    post,
    path = "/api/access/requests",
    tag = "Access",
    request_body = CreateRequestBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 201, description = "Join request created", body = JoinRequest),
        (status = 400, description = "The request is not legal from the caller's standing", body = ErrorBody),
        (status = 409, description = "A request is already pending, or access is already granted", body = ErrorBody),
        (status = 429, description = "Too many requests in the current window (rate limit configured by the operator; default unlimited)", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn create_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateRequestBody>,
) -> ApiResult<(StatusCode, Json<JoinRequest>)> {
    let params = access_service::CreateJoinRequestParams {
        profile_id: ProfileId::from(auth.0.profile().id),
        message: body.message,
        source: body.source,
        accepted_terms_version: body.accepted_terms_version,
    };

    let request = access_service::create_join_request(
        &state.pool,
        params,
        state.config.rate_limit.and_then(|r| r.create_request),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(request)))
}

/// Check your join request status
#[utoipa::path(
    get,
    path = "/api/access/requests/me",
    tag = "Access",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Own join request, or null if none exists", body = Option<JoinRequest>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn get_own_request(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Option<JoinRequest>>> {
    let request =
        access_service::get_own_request(&state.pool, ProfileId::from(auth.0.profile().id)).await?;
    Ok(Json(request))
}

/// Withdraw your pending join request
#[utoipa::path(
    delete,
    path = "/api/access/requests/me",
    tag = "Access",
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Pending join request withdrawn"),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "No pending join request to withdraw", body = ErrorBody),
    )
)]
pub async fn withdraw_request(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<StatusCode> {
    access_service::withdraw_request(&state.pool, ProfileId::from(auth.0.profile().id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Spec D15. On the auth-only router, NOT the gated one: a revoked principal cannot pass the
// system-access gate, and being able to ask for reconsideration is the whole point. The review
// is an inbox signal only — it never feeds the admission decision.
/// Ask an admin to reconsider a revocation
///
/// A revoked principal can call this without passing the system-access gate — being able to ask for reconsideration is the point.
///
/// The review is an inbox signal for administrators. It never feeds the admission decision by itself.
#[utoipa::path(
    post,
    path = "/api/access/reviews",
    tag = "Access",
    request_body = CreateReviewBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 201, description = "Review request recorded"),
        (status = 400, description = "Only a revoked principal may request review", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn create_review_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateReviewBody>,
) -> ApiResult<StatusCode> {
    access_service::create_review_request(
        &state.pool,
        access_service::CreateReviewRequestParams {
            profile_id: ProfileId::from(auth.0.profile().id),
            message: body.message,
        },
    )
    .await?;
    Ok(StatusCode::CREATED)
}

/// Read public system settings
#[utoipa::path(
    get,
    path = "/api/access/settings",
    tag = "Access",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Public system settings", body = PublicSystemSettings),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn get_settings(State(state): State<AppState>) -> ApiResult<Json<PublicSystemSettings>> {
    access_service::get_public_settings(&state.pool)
        .await
        .map(Json)
}

// ---------------------------------------------------------------------------
// Admin endpoints (the admin router — `routes/admin.rs`)
//
// The operator surface. Documented in the OpenAPI contract under the `Admin` tag: an admin
// door is not a secret, and a public repository cannot make it one. What protects it is the
// gate, not the omission — which is why it is pinned by tests rather than hidden.
//
// Their authz is the `&SystemAdmin` proof each dispatches with (admin-authz
// enclosure, spec §3): the handler mints it once via `require_system_admin` and
// the service fn requires it in its signature. There is no handler-side
// `is_system_admin` any more — the requirement lives in the service type, so
// both surfaces enforce it identically (the F-3 posture; see
// `audit-handler-authz-drift`).
// ---------------------------------------------------------------------------

/// GET /api/access/admin/requests — list pending join requests (admin only).
#[utoipa::path(
    get,
    operation_id = "admin_list_join_requests",
    summary = "List pending join requests",
    description = "Every join request still awaiting a decision, with the requesting profile's handle, display name and email. Requires a system admin.",
    path = "/api/access/admin/requests",
    tag = "Admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Pending join requests, with the requesting profile's identity", body = Vec<JoinRequestWithProfile>),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
)
)]
pub async fn list_pending(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<JoinRequestWithProfile>>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::list_pending_requests(&state.pool, &admin)
        .await
        .map(Json)
}

/// GET /api/access/admin/requests/count — how many join requests are outstanding (admin only).
///
/// [`list_pending`] without the rows, for `temper warmup`. Same admin proof, so a caller who may
/// not read the queue still gets a `403` — never a `0`, which would tell them the queue is empty
/// while refusing to let them see it.
#[utoipa::path(
    get,
    operation_id = "admin_count_join_requests",
    summary = "Count pending join requests",
    description = "How many join requests are awaiting a decision, without the rows. A caller who may not read the queue gets 403, never a zero. Requires a system admin.",
    path = "/api/access/admin/requests/count",
    tag = "Admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "How many join requests are pending", body = QueueCount),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
)
)]
pub async fn count_pending(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<QueueCount>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::count_pending_requests(&state.pool, &admin)
        .await
        .map(|count| Json(QueueCount { count }))
}

/// PATCH /api/access/admin/requests/:id — approve or reject a join request (admin only).
#[utoipa::path(
    patch,
    operation_id = "admin_review_join_request",
    summary = "Approve or reject a join request",
    description = "Records the decision on a pending join request, with an optional note. Requires a system admin.",
    path = "/api/access/admin/requests/{id}",
    tag = "Admin",
    params(("id" = Uuid, Path, description = "Join request ID")),
    request_body = ReviewRequestBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The reviewed join request", body = JoinRequest),
        (status = 400, description = "The decision is not a legal review outcome", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such join request", body = ErrorBody),
)
)]
pub async fn review_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(request_id): Path<Uuid>,
    Json(body): Json<ReviewRequestBody>,
) -> ApiResult<Json<JoinRequest>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    let params = access_service::ReviewRequestParams {
        request_id,
        decision: body.status,
        decision_note: body.decision_note,
    };

    access_service::review_request(&state.pool, &admin, params)
        .await
        .map(Json)
}

/// GET /api/access/admin/reviews — list undecided reconsideration requests.
///
/// The read half of the inbox `kb_principal_review_requests` always described itself as and never
/// had. Same operator-only posture as the join-request queue above: the `&SystemAdmin` proof is
/// minted here and required by the service.
#[utoipa::path(
    get,
    operation_id = "admin_list_reviews",
    summary = "List open reconsideration requests",
    description = "Reconsideration requests that have not been closed, with the asking principal's identity. Requires a system admin.",
    path = "/api/access/admin/reviews",
    tag = "Admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Open reconsideration requests, with the asking principal's identity", body = Vec<ReviewRequestWithProfile>),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
)
)]
pub async fn list_reviews(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<ReviewRequestWithProfile>>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::list_open_review_requests(&state.pool, &admin)
        .await
        .map(Json)
}

/// GET /api/access/admin/reviews/count — how many reconsiderations are open (admin only).
///
/// [`list_reviews`] without the rows. Same admin proof, same `403`-not-`0` rule as its neighbour.
#[utoipa::path(
    get,
    operation_id = "admin_count_reviews",
    summary = "Count open reconsideration requests",
    description = "How many reconsideration requests are open, without the rows. A caller who may not read the inbox gets 403, never a zero. Requires a system admin.",
    path = "/api/access/admin/reviews/count",
    tag = "Admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "How many reconsideration requests are open", body = QueueCount),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
)
)]
pub async fn count_reviews(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<QueueCount>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::count_open_review_requests(&state.pool, &admin)
        .await
        .map(|count| Json(QueueCount { count }))
}

/// PATCH /api/access/admin/reviews/:id — record that a reconsideration was handled.
///
/// Returns `204`: there is no updated resource worth handing back, because closing changes nothing
/// the caller can act on further. It moves **no** standing — readmitting a principal is
/// `POST /api/access/admin/principals/{id}/approve`, deliberately a different call.
#[utoipa::path(
    patch,
    operation_id = "admin_close_review",
    summary = "Close a reconsideration request",
    description = "Records that a reconsideration request was handled. It changes no standing: readmitting a principal is a separate call, `POST /api/access/admin/principals/{id}/approve`. Requires a system admin.",
    path = "/api/access/admin/reviews/{id}",
    tag = "Admin",
    params(("id" = Uuid, Path, description = "Reconsideration request ID")),
    request_body = CloseReviewBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Reconsideration recorded as handled; no standing moved"),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such open reconsideration request", body = ErrorBody),
)
)]
pub async fn close_review(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(request_id): Path<Uuid>,
    Json(body): Json<CloseReviewBody>,
) -> ApiResult<StatusCode> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    let params = access_service::CloseReviewRequestParams {
        request_id,
        decision_note: body.decision_note,
    };

    access_service::close_review_request(&state.pool, &admin, params).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/access/admin/settings — read FULL system settings (admin only).
///
/// Unlike the public `GET /api/access/settings`, this returns `gating_team_slug`
/// and `updated`, which an admin needs to administer the gate.
#[utoipa::path(
    get,
    operation_id = "admin_get_settings",
    summary = "Read full system settings",
    description = "The full instance settings, including the gating team slug that the public settings read withholds. Requires a system admin.",
    path = "/api/access/admin/settings",
    tag = "Admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Full system settings, including the gating team slug", body = SystemSettings),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
)
)]
pub async fn get_admin_settings(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<SystemSettings>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::admin_get_settings(&state.pool, &admin)
        .await
        .map(Json)
}

/// PATCH /api/access/admin/settings — partial update of system settings (admin only).
#[utoipa::path(
    patch,
    operation_id = "admin_update_settings",
    summary = "Update system settings",
    description = "Partial update: each field present overwrites its setting, each field absent is left unchanged. Requires a system admin.",
    path = "/api/access/admin/settings",
    tag = "Admin",
    request_body = UpdateSettingsRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Settings after the partial update", body = SystemSettings),
        (status = 400, description = "Invalid settings value", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
)
)]
pub async fn update_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpdateSettingsRequest>,
) -> ApiResult<Json<SystemSettings>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::update_system_settings(&state.pool, &admin, &body)
        .await
        .map(Json)
}

/// POST /api/access/admin/promote — promote a profile to system admin (admin only).
///
/// Grants `kb_principal_governance` + `approved` standing (the real admin-ness
/// under D11). `team_id` omitted ⇒ the configured gating team for the retained
/// side-effect `owner` row.
#[utoipa::path(
    post,
    operation_id = "admin_promote",
    summary = "Promote a profile to system admin",
    description = "Grants the system-admin governance grant and approved standing. Also adds an `owner` row on the given team (the configured gating team when omitted); that row confers no authority by itself. Requires a system admin.",
    path = "/api/access/admin/promote",
    tag = "Admin",
    request_body = PromoteAdminRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Profile promoted; the side-effect team membership row", body = TeamMemberRow),
        (status = 400, description = "The profile or team cannot be promoted into", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
)
)]
pub async fn promote_admin(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<PromoteAdminRequest>,
) -> ApiResult<Json<TeamMemberRow>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::promote_admin(&state.pool, &admin, body.profile_id, body.team_id)
        .await
        .map(Json)
}

/// POST /api/access/admin/demote — revoke a profile's system-admin grant (admin only).
///
/// The manual governance twin of `promote_admin`; the automatic path is demotion-by-transition in
/// `standing_service::apply` (Revoke/Deactivate demote). Like every admin handler, it mints the
/// `&SystemAdmin` proof via `require_system_admin` and `access_service::demote_admin` requires it
/// in its signature, so the gate is the service type (the F-3 posture `audit-handler-authz-drift`
/// pins), not a handler-side `is_system_admin` check.
#[utoipa::path(
    post,
    operation_id = "admin_demote",
    summary = "Revoke a profile's system-admin grant",
    description = "Removes the system-admin governance grant from a profile. Idempotent. Requires a system admin.",
    path = "/api/access/admin/demote",
    tag = "Admin",
    request_body = DemoteAdminRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "System-admin governance grant revoked (idempotent)"),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
)
)]
pub async fn demote_admin(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<DemoteAdminRequest>,
) -> ApiResult<StatusCode> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::demote_admin(&state.pool, &admin, ProfileId::from(body.profile_id)).await?;
    Ok(StatusCode::OK)
}

// ---------------------------------------------------------------------------
// The admin standing acts (Task 13) — operator-only, documented under the `Admin` tag like
// their neighbours above.
//
// Their authz IS the service signature: each dispatches to an `access_service`
// fn that requires a `&SystemAdmin` proof (admin-authz enclosure, spec §3),
// minted here by `require_system_admin`. Because the requirement lives in the
// service type — not a handler-side `is_system_admin` — both surfaces enforce it
// identically and a future MCP tool cannot bypass it (the F-3 posture; see
// `audit-handler-authz-drift`). The handler mints the proof, then dispatches.
// ---------------------------------------------------------------------------

/// POST /api/access/admin/principals/:id/approve — admit a principal directly (admin only).
#[utoipa::path(
    post,
    operation_id = "admin_approve_principal",
    summary = "Approve a principal",
    description = "Admits a principal directly and closes any open reconsideration request they hold. Requires a system admin.",
    path = "/api/access/admin/principals/{id}/approve",
    tag = "Admin",
    params(("id" = Uuid, Path, description = "Profile ID of the principal")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Principal approved"),
        (status = 400, description = "Approval is not a legal transition from the principal's standing", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 409, description = "The principal is already approved", body = ErrorBody),
)
)]
pub async fn approve_principal(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(profile_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::admin_approve(&state.pool, &admin, ProfileId::from(profile_id)).await?;
    Ok(StatusCode::OK)
}

/// POST /api/access/admin/auto-join/reconcile — converge every auto-join team's roster to the
/// standing-approved humans, never machines (admin only). Returns the (team, profile) pairs added plus
/// the touched teams that also carry SAML group mappings (whose new native rows pre-empt
/// IdP role assertions); an empty `added` means the instance was already converged.
#[utoipa::path(
    post,
    operation_id = "admin_reconcile_auto_join",
    summary = "Reconcile auto-join team rosters",
    description = "Adds every approved person (never a machine principal) missing from an auto-join team and reports each (team, profile) pair added, plus the touched teams that also carry SAML group mappings (whose new native memberships take precedence over IdP role assertions). An empty `added` means nothing needed adding. Requires a system admin.",
    path = "/api/access/admin/auto-join/reconcile",
    tag = "Admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The roster pairs added (empty when already converged) and the SAML-mapped teams touched", body = ReconcileAutoJoinOutcome),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
)
)]
pub async fn reconcile_auto_join(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<ReconcileAutoJoinOutcome>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    let outcome = access_service::reconcile_auto_join(&state.pool, &admin).await?;
    Ok(Json(outcome))
}

/// POST /api/access/admin/principals/:id/revoke — revoke a principal's admission (admin only).
#[utoipa::path(
    post,
    operation_id = "admin_revoke_principal",
    summary = "Revoke a principal's admission",
    description = "Revokes a principal's admission, with a required reason that is recorded. Requires a system admin.",
    path = "/api/access/admin/principals/{id}/revoke",
    tag = "Admin",
    params(("id" = Uuid, Path, description = "Profile ID of the principal")),
    request_body = RevokePrincipalBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Principal's admission revoked"),
        (status = 400, description = "Revocation is not a legal transition from the principal's standing", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 409, description = "The transition conflicts with the principal's current standing", body = ErrorBody),
)
)]
pub async fn revoke_principal(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(profile_id): Path<Uuid>,
    Json(body): Json<RevokePrincipalBody>,
) -> ApiResult<StatusCode> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::admin_revoke(
        &state.pool,
        &admin,
        ProfileId::from(profile_id),
        body.reason,
    )
    .await?;
    Ok(StatusCode::OK)
}

/// POST /api/access/admin/principals/:id/deactivate — deactivate a principal (admin only).
#[utoipa::path(
    post,
    operation_id = "admin_deactivate_principal",
    summary = "Deactivate a principal",
    description = "Deactivates a principal. Requires a system admin.",
    path = "/api/access/admin/principals/{id}/deactivate",
    tag = "Admin",
    params(("id" = Uuid, Path, description = "Profile ID of the principal")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Principal deactivated"),
        (status = 400, description = "Deactivation is not a legal transition from the principal's standing", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 409, description = "The transition conflicts with the principal's current standing", body = ErrorBody),
)
)]
pub async fn deactivate_principal(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(profile_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::admin_deactivate(&state.pool, &admin, ProfileId::from(profile_id)).await?;
    Ok(StatusCode::OK)
}

/// POST /api/access/admin/principals/:id/reactivate — restore a deactivated principal (admin only).
#[utoipa::path(
    post,
    operation_id = "admin_reactivate_principal",
    summary = "Reactivate a principal",
    description = "Restores a deactivated principal. Requires a system admin.",
    path = "/api/access/admin/principals/{id}/reactivate",
    tag = "Admin",
    params(("id" = Uuid, Path, description = "Profile ID of the principal")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Principal reactivated"),
        (status = 400, description = "Reactivation is not a legal transition from the principal's standing", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 409, description = "The transition conflicts with the principal's current standing", body = ErrorBody),
)
)]
pub async fn reactivate_principal(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(profile_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::admin_reactivate(&state.pool, &admin, ProfileId::from(profile_id)).await?;
    Ok(StatusCode::OK)
}
