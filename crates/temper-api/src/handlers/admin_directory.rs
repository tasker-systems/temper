//! Handlers for the operator directory — `GET /api/access/admin/profiles` (the list, §5) and
//! `GET /api/access/admin/profiles/{profile_id}` (the state card, §6).
//!
//! Out of the OpenAPI contract (plain `.route()` mounting), like the rest of
//! `/api/access/admin/*` — its paths are on the allowlist in
//! `.github/scripts/check-openapi-routes.sh`.
//!
//! **Authorization is the service signature, not this file.** Every directory function takes a
//! sealed `&SystemAdmin` (minted here by `require_system_admin` and dispatched immediately), so
//! CLI/MCP/API enforce identically and the gate runs BEFORE any existence lookup — a non-admin
//! asking for a nonexistent profile gets the same plain 403 as for a real one, so absence never
//! leaks below the gate (spec §7, pinned by test).
//!
//! One contract subtlety lives here because it changes the response SHAPE: `?email=` on the
//! list route resolves an address EXACTLY (case-insensitive, verified) and answers the single
//! matching state card — the one place a human-controlled address becomes a target UUID
//! (spec §6, review C1). It shares the route with the page read because it shares its path,
//! nothing else: it is dispatched first, and it refuses to coexist with `email_contains`.

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;

use temper_core::types::admin::{
    AdminDirectoryListResponse, AdminProfileCard, AdminProfilesListQuery,
};
use temper_core::types::ids::ProfileId;

use crate::middleware::auth::AuthUser;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::admin_directory_service;
use temper_services::state::AppState;

/// GET /api/access/admin/profiles — the directory list, or the state card when `?email=`
/// resolves exactly one verified address.
pub async fn list_profiles(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<AdminProfilesListQuery>,
) -> ApiResult<Response> {
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
        return Ok(Json(card).into_response());
    }

    let page: AdminDirectoryListResponse =
        admin_directory_service::list_profiles(&state.pool, &admin, &q).await?;
    Ok(Json(page).into_response())
}

/// GET /api/access/admin/profiles/{profile_id} — the principal state card.
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
