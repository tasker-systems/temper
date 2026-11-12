//! `/api/admin/saml/group-mappings` — create and remove a SAML group mapping, a system admin's
//! recorded act (`saml_mapping_service`). Admin group: a machine is refused at the tier, and the
//! handler mints the `SystemAdmin` proof before the service runs.

use axum::extract::{Query, State};
use axum::Json;

use temper_core::types::saml_mapping::{
    CreateSamlGroupMappingOutcome, CreateSamlGroupMappingRequest, RemoveSamlGroupMappingOutcome,
    RemoveSamlGroupMappingParams,
};
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::saml_mapping_service;
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;

/// `POST /api/admin/saml/group-mappings` — map an IdP group to a role on a team.
#[utoipa::path(
    post,
    operation_id = "admin_create_saml_group_mapping",
    summary = "Create a SAML group mapping",
    description = "Maps a group the IdP asserts to a role on a team: at each login, the IdP reconcile gives everyone in the group that role. The ceiling is `maintainer`. An `owner` role, or an owner, maintainer or member role onto a team with a pool at or beneath it (temper-system, the gating team or an auto-join team, live or deleted), needs `confirm_elevated: true` (a watcher mapping never does); without it the request is refused, naming why, and nothing is written. Recorded as `saml_group_mapping_changed`. The same mapping again records nothing and answers `created: false`. Requires a system admin.",
    path = "/api/admin/saml/group-mappings",
    tag = "Admin",
    request_body = CreateSamlGroupMappingRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Created, or already so", body = CreateSamlGroupMappingOutcome),
        (status = 400, description = "Confirmation required (an `owner` role, or an authoring role onto a team with a pool at or beneath it, without `confirm_elevated`); a personal team; or an empty idp_key or group. Nothing is written", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = ErrorBody),
        (status = 404, description = "No IdP has this idp_key, or the team does not exist or is soft-deleted", body = ErrorBody),
        (status = 409, description = "The group already maps to the team with another role; remove that mapping first", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateSamlGroupMappingRequest>,
) -> ApiResult<Json<CreateSamlGroupMappingOutcome>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    saml_mapping_service::create(&state.pool, &admin, &body)
        .await
        .map(Json)
}

/// `DELETE /api/admin/saml/group-mappings` — remove a SAML group mapping.
#[utoipa::path(
    delete,
    operation_id = "admin_remove_saml_group_mapping",
    summary = "Remove a SAML group mapping",
    description = "Removes one mapping. Memberships it already conferred are left in place: the IdP reconcile governs them, and at each member's next login removes those no remaining mapping confers. The answer counts the team's IdP memberships and says so. Recorded as `saml_group_mapping_changed`. Requires a system admin.",
    path = "/api/admin/saml/group-mappings",
    tag = "Admin",
    params(RemoveSamlGroupMappingParams),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Removed", body = RemoveSamlGroupMappingOutcome),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = ErrorBody),
        (status = 404, description = "No such mapping", body = ErrorBody),
    )
)]
pub async fn remove(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<RemoveSamlGroupMappingParams>,
) -> ApiResult<Json<RemoveSamlGroupMappingOutcome>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    saml_mapping_service::remove(&state.pool, &admin, &params)
        .await
        .map(Json)
}
