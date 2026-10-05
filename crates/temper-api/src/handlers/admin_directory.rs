//! Handlers for the operator directory — `GET /api/access/admin/profiles` (the list, §5) and
//! `GET /api/access/admin/profiles/{profile_id}` (the state card, §6).
//!
//! Documented in the OpenAPI contract under the `Admin` tag, like the rest of
//! `/api/access/admin/*` (mounted by `routes/admin.rs`).
//!
//! **Authorization is the service signature, not this file.** Every directory function takes a
//! sealed `&SystemAdmin` (minted here by `require_system_admin` and dispatched immediately), so
//! CLI/MCP/API enforce identically and the gate runs BEFORE any existence lookup — a non-admin
//! asking for a nonexistent profile gets the same plain 403 as for a real one, so absence never
//! leaks below the gate (spec §7, pinned by test).
//!
//! That uniform-403 claim is scoped to WELL-FORMED requests, deliberately: axum's extractors
//! reject a malformed query (`?limit=abc`) or a non-UUID path segment with 400/422 before the
//! gate ever runs. An extractor rejection names only the caller's own malformed input — it
//! carries no existence information — so the scope is a fact about request parsing, not a
//! weakening of the gate.
//!
//! One contract subtlety lives here because it changes the response SHAPE: `?email=` on the
//! list route resolves an address EXACTLY (case-insensitive, verified) and answers the single
//! matching state card — the one place a human-controlled address becomes a target UUID
//! (spec §6, review C1). It shares the route with the page read because it shares its path,
//! nothing else: it is dispatched first, and it refuses to coexist with `email_contains`.

use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use temper_core::types::admin::{
    AdminDirectoryListResponse, AdminProfileCard, AdminProfilesListQuery,
};
use temper_core::types::ids::ProfileId;

use crate::middleware::auth::AuthUser;
use temper_core::types::admin::AdminProfilesListAnswer;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::services::admin_directory_service;
use temper_services::state::AppState;

/// GET /api/access/admin/profiles — the directory list, or the state card when `?email=`
/// resolves exactly one verified address.
#[utoipa::path(
    get,
    operation_id = "admin_list_profiles",
    summary = "List profiles in the operator directory",
    description = "A filtered, paged directory of profiles with admission state, admin status and default verified email. With `email`, resolves exactly one verified address and answers that profile's state card instead of a page. Requires a system admin.",
    path = "/api/access/admin/profiles",
    tag = "Admin",
    params(AdminProfilesListQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "A directory page, or — when `email` is given — the one matching profile's state card", body = AdminProfilesListAnswer),
        (status = 400, description = "Invalid filter, or both `email` and `email_contains` given", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "`email` matched no profile, or more than one", body = ErrorBody),
    )
)]
pub async fn list_profiles(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<AdminProfilesListQuery>,
) -> ApiResult<Json<AdminProfilesListAnswer>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;

    // ?email= is the identity-resolution act: exact, server-side, ambiguity-refusing. It is a
    // different response shape from the page, so it never reaches the list query.
    if let Some(email) = q.email.as_deref() {
        if q.email_contains.is_some() {
            return Err(ApiError::BadRequest(
                "pass either email or email_contains, not both".to_string(),
            ));
        }
        let card: AdminProfileCard =
            admin_directory_service::profile_card_by_email(&state.pool, &admin, email).await?;
        return Ok(Json(AdminProfilesListAnswer::Card(Box::new(card))));
    }

    let page: AdminDirectoryListResponse =
        admin_directory_service::list_profiles(&state.pool, &admin, &q).await?;
    Ok(Json(AdminProfilesListAnswer::Page(page)))
}

/// GET /api/access/admin/profiles/{profile_id} — the principal state card.
#[utoipa::path(
    get,
    operation_id = "admin_show_profile",
    summary = "Show a profile's state card",
    description = "One profile's admission state, governance, identity links, team memberships, pending invitations and open queue items. Requires a system admin.",
    path = "/api/access/admin/profiles/{profile_id}",
    tag = "Admin",
    params(("profile_id" = Uuid, Path, description = "Profile ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The principal state card", body = AdminProfileCard),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such profile", body = ErrorBody),
    )
)]
pub async fn show_profile(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(profile_id): Path<Uuid>,
) -> ApiResult<Json<AdminProfileCard>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    let card =
        admin_directory_service::profile_card(&state.pool, &admin, ProfileId::from(profile_id))
            .await?;
    Ok(Json(card))
}
