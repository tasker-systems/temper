//! Handlers for the system access gate endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::access_gate::{
    JoinRequest, JoinRequestStatus, JoinRequestWithProfile, PublicSystemSettings, SystemSettings,
};
use temper_core::types::admin::{DemoteAdminRequest, PromoteAdminRequest, UpdateSettingsRequest};
use temper_core::types::ids::ProfileId;
use temper_core::types::team::TeamMemberRow;

use crate::middleware::auth::AuthUser;
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::access_service;
use temper_services::state::AppState;

// ---------------------------------------------------------------------------
// Request body types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateRequestBody {
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateReviewBody {
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewRequestBody {
    pub status: JoinRequestStatus,
    pub decision_note: Option<String>,
}

// ---------------------------------------------------------------------------
// Public endpoints (auth_only router)
// ---------------------------------------------------------------------------

/// POST /api/access/requests — submit a join request for the gating team.
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

    let request = access_service::create_join_request(&state.pool, params).await?;
    Ok((StatusCode::CREATED, Json(request)))
}

/// GET /api/access/requests/me — check own join request status.
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

/// DELETE /api/access/requests/me — withdraw a pending join request.
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

/// POST /api/access/reviews — a revoked principal asks an admin to reconsider (spec D15).
///
/// On the auth-only router, NOT the gated one: a revoked principal cannot pass the system-access
/// gate, and being able to ask for reconsideration is the whole point. The review is an inbox
/// signal only — it never feeds the admission decision.
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

/// GET /api/access/settings — read public system settings.
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
// Admin endpoints (gated router)
//
// These five handlers (`list_pending`, `review_request`, `get_admin_settings`,
// `update_settings`, `promote_admin`) are DELIBERATELY left without
// `#[utoipa::path]` annotations: they are an operator-only surface, not part of
// the documented client API. A library caller requesting access to their own
// instance uses the four self-service handlers above; the operator surface is
// intentionally excluded from the OpenAPI spec.
//
// Their authz is the `&SystemAdmin` proof each dispatches with (admin-authz
// enclosure, spec §3): the handler mints it once via `require_system_admin` and
// the service fn requires it in its signature. There is no handler-side
// `is_system_admin` any more — the requirement lives in the service type, so
// both surfaces enforce it identically (the F-3 posture; see
// `audit-handler-authz-drift`).
// ---------------------------------------------------------------------------

/// GET /api/access/admin/requests — list pending join requests (admin only).
pub async fn list_pending(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<JoinRequestWithProfile>>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::list_pending_requests(&state.pool, &admin)
        .await
        .map(Json)
}

/// PATCH /api/access/admin/requests/:id — approve or reject a join request (admin only).
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

/// GET /api/access/admin/settings — read FULL system settings (admin only).
///
/// Unlike the public `GET /api/access/settings`, this returns `gating_team_slug`
/// and `updated`, which an admin needs to administer the gate.
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

/// POST /api/access/admin/promote — grant a profile `owner` on a team (admin only).
///
/// `team_id` omitted ⇒ the configured gating team (mints a second system admin).
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
/// `standing_service::apply` (Revoke/Deactivate demote). Unlike its older sibling above, it carries
/// NO handler-side authz: the gate lives in `access_service::demote_admin` (the F-3 posture the
/// `audit-handler-authz-drift` tripwire pins). The handler extracts actor + subject and dispatches.
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
// The admin standing acts (Task 13) — operator-only, UNDOCUMENTED like their
// neighbours above (plain `.route()`, no `#[utoipa::path]`, allowlisted in
// `.github/scripts/check-openapi-routes.sh`).
//
// Their authz IS the service signature: each dispatches to an `access_service`
// fn that requires a `&SystemAdmin` proof (admin-authz enclosure, spec §3),
// minted here by `require_system_admin`. Because the requirement lives in the
// service type — not a handler-side `is_system_admin` — both surfaces enforce it
// identically and a future MCP tool cannot bypass it (the F-3 posture; see
// `audit-handler-authz-drift`). The handler mints the proof, then dispatches.
// ---------------------------------------------------------------------------

/// Body for `POST /api/access/admin/principals/{id}/revoke`.
#[derive(Deserialize)]
pub struct RevokePrincipalBody {
    /// Required. It rides the log and the ledger, and a later review's reviewer needs it (D15).
    pub reason: String,
}

/// POST /api/access/admin/principals/:id/approve — admit a principal directly (admin only).
pub async fn approve_principal(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(profile_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::admin_approve(&state.pool, &admin, ProfileId::from(profile_id)).await?;
    Ok(StatusCode::OK)
}

/// POST /api/access/admin/principals/:id/revoke — revoke a principal's admission (admin only).
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
pub async fn reactivate_principal(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(profile_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::admin_reactivate(&state.pool, &admin, ProfileId::from(profile_id)).await?;
    Ok(StatusCode::OK)
}
