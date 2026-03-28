use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;

use temper_core::types::{Profile, ProfileAuthLink};

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::profile_service;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/profile",
    tag = "Profile",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Current authenticated profile", body = Profile),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn get(State(state): State<AppState>, auth: AuthUser) -> ApiResult<Json<Profile>> {
    profile_service::get_by_id(&state.pool, auth.0.profile.id)
        .await
        .map(Json)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProfileUpdateRequest {
    pub display_name: Option<String>,
    pub preferences: Option<Value>,
    pub vault_config: Option<Value>,
}

#[utoipa::path(
    patch,
    path = "/api/profile",
    tag = "Profile",
    request_body = ProfileUpdateRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Updated profile", body = Profile),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
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

#[utoipa::path(
    get,
    path = "/api/profile/auth-links",
    tag = "Profile",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Linked auth providers", body = Vec<ProfileAuthLink>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list_auth_links(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<ProfileAuthLink>>> {
    profile_service::list_auth_links(&state.pool, auth.0.profile.id)
        .await
        .map(Json)
}
