use axum::extract::State;
use axum::Json;

use crate::error::ApiResult;
use crate::middleware::auth::AuthUser;
use crate::services::sync_service;
use crate::state::AppState;

use temper_core::types::sync::{
    SyncCompleteRequest, SyncCompleteResponse, SyncManifestResponse, SyncStatusRequest,
    SyncStatusResponse,
};

pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SyncStatusRequest>,
) -> ApiResult<Json<SyncStatusResponse>> {
    sync_service::compute_sync_diff(&state.pool, auth.0.profile.id, body)
        .await
        .map(Json)
}

pub async fn complete(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SyncCompleteRequest>,
) -> ApiResult<Json<SyncCompleteResponse>> {
    sync_service::complete_sync_round(&state.pool, auth.0.profile.id, body)
        .await
        .map(Json)
}

pub async fn manifest(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<SyncManifestResponse>> {
    sync_service::fetch_manifest(&state.pool, auth.0.profile.id)
        .await
        .map(Json)
}
