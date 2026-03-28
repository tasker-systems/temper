use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use temper_core::types::{Profile, ProfileAuthLink};

use crate::error::ApiResult;
use crate::middleware::auth::AuthUser;
use crate::services::profile_service;
use crate::state::AppState;

pub async fn get(State(state): State<AppState>, auth: AuthUser) -> ApiResult<Json<Profile>> {
    profile_service::get_by_id(&state.pool, auth.0.profile.id)
        .await
        .map(Json)
}

#[derive(Debug, Deserialize)]
pub struct ProfileUpdateRequest {
    pub display_name: Option<String>,
    pub preferences: Option<Value>,
    pub vault_config: Option<Value>,
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ProfileUpdateRequest>,
) -> ApiResult<Json<Profile>> {
    profile_service::update(
        &state.pool,
        auth.0.profile.id,
        req.display_name.as_deref(),
        req.preferences.as_ref(),
        req.vault_config.as_ref(),
    )
    .await
    .map(Json)
}

pub async fn list_auth_links(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<ProfileAuthLink>>> {
    profile_service::list_auth_links(&state.pool, auth.0.profile.id)
        .await
        .map(Json)
}
