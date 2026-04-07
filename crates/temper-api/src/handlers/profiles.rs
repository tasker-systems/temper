use axum::extract::State;
use axum::Json;

use temper_core::types::access_gate::Entitlements;
use temper_core::types::{Profile, ProfileAuthLink, ProfileUpdateRequest};

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::{access_service, profile_service};
use crate::state::AppState;

#[derive(serde::Serialize)]
pub struct ProfileWithEntitlements {
    #[serde(flatten)]
    pub profile: Profile,
    pub entitlements: Entitlements,
}

#[utoipa::path(
    get,
    path = "/api/profile",
    tag = "Profile",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Current authenticated profile with entitlements"),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<ProfileWithEntitlements>> {
    let profile = profile_service::get_by_id(&state.pool, auth.0.profile.id).await?;
    let entitlements = access_service::get_entitlements(&state.pool, auth.0.profile.id).await?;

    Ok(Json(ProfileWithEntitlements {
        profile,
        entitlements,
    }))
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
    profile_service::validate_preferences_size(req.preferences.as_ref())?;

    let vault_config_value = req
        .vault_config
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| ApiError::BadRequest(format!("Invalid vault_config: {e}")))?;

    profile_service::update(
        &state.pool,
        auth.0.profile.id,
        req.display_name.as_deref(),
        req.preferences.as_ref(),
        vault_config_value.as_ref(),
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
