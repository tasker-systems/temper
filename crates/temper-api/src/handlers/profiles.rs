use axum::extract::State;
use axum::Json;

use temper_core::types::access_gate::Entitlements;
use temper_core::types::ids::ProfileId;
use temper_core::types::{Profile, ProfileAuthLink, ProfileUpdateRequest};

use crate::middleware::auth::AuthUser;
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::{access_service, profile_service};
use temper_services::state::AppState;

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct ProfileWithEntitlements {
    #[serde(flatten)]
    pub profile: Profile,
    pub entitlements: Entitlements,
}

#[utoipa::path(
    get,
    operation_id = "get_profile",
    path = "/api/profile",
    tag = "Profile",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Current authenticated profile with entitlements", body = ProfileWithEntitlements),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<ProfileWithEntitlements>> {
    let profile =
        profile_service::get_by_id(&state.pool, ProfileId::from(auth.0.profile().id)).await?;
    let entitlements =
        access_service::get_entitlements(&state.pool, ProfileId::from(auth.0.profile().id)).await?;

    Ok(Json(ProfileWithEntitlements {
        profile,
        entitlements,
    }))
}

#[utoipa::path(
    patch,
    operation_id = "update_profile",
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

    // `req.vault_config` is intentionally ignored: it is substrate-dropped
    // (synthesized on read), so there is nothing to persist.
    profile_service::update(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        req.display_name.as_deref(),
        req.preferences.as_ref(),
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
    profile_service::list_auth_links(&state.pool, ProfileId::from(auth.0.profile().id))
        .await
        .map(Json)
}
