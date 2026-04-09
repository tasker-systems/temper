//! Handlers for the system access gate endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::access_gate::{
    JoinRequest, JoinRequestStatus, JoinRequestWithProfile, PublicSystemSettings,
};

use crate::error::{ApiError, ApiResult};
use crate::middleware::auth::AuthUser;
use crate::services::access_service;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request body types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateRequestBody {
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
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
pub async fn create_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateRequestBody>,
) -> ApiResult<(StatusCode, Json<JoinRequest>)> {
    let params = access_service::CreateJoinRequestParams {
        profile_id: auth.0.profile.id,
        message: body.message,
        source: body.source,
        accepted_terms_version: body.accepted_terms_version,
    };

    let request = access_service::create_join_request(&state.pool, params).await?;
    Ok((StatusCode::CREATED, Json(request)))
}

/// GET /api/access/requests/me — check own join request status.
pub async fn get_own_request(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Option<JoinRequest>>> {
    let request = access_service::get_own_request(&state.pool, auth.0.profile.id).await?;
    Ok(Json(request))
}

/// DELETE /api/access/requests/me — withdraw a pending join request.
pub async fn withdraw_request(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<StatusCode> {
    access_service::withdraw_request(&state.pool, auth.0.profile.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/access/settings — read public system settings.
pub async fn get_settings(State(state): State<AppState>) -> ApiResult<Json<PublicSystemSettings>> {
    access_service::get_public_settings(&state.pool)
        .await
        .map(Json)
}

// ---------------------------------------------------------------------------
// Admin endpoints (gated router, handler-level admin check)
// ---------------------------------------------------------------------------

/// GET /api/access/admin/requests — list pending join requests (admin only).
pub async fn list_pending(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<JoinRequestWithProfile>>> {
    let is_admin = access_service::is_system_admin(&state.pool, auth.0.profile.id).await?;
    if !is_admin {
        return Err(ApiError::Forbidden);
    }

    access_service::list_pending_requests(&state.pool)
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
    let is_admin = access_service::is_system_admin(&state.pool, auth.0.profile.id).await?;
    if !is_admin {
        return Err(ApiError::Forbidden);
    }

    let params = access_service::ReviewRequestParams {
        request_id,
        reviewer_profile_id: auth.0.profile.id,
        decision: body.status,
        decision_note: body.decision_note,
    };

    access_service::review_request(&state.pool, params)
        .await
        .map(Json)
}
