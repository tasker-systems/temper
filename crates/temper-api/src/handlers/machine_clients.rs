//! Operator-only machine-client registration. Out of the OpenAPI contract (plain
//! `.route()` mounting), like `/api/access/admin/*`.
//!
//! **The `is_system_admin` check here is load-bearing, not defense-in-depth.** Production
//! runs `access_mode = 'open'`, under which `has_system_access` is true for every profile,
//! so `require_system_access` on the gated router admits everyone. This check is the only
//! thing protecting these endpoints (D12).

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{
    IssueMachineRequest, IssuedMachineCredential, MachineClient, ProvisionMachineRequest,
    RebindMachineRequest, RotateSecretRequest,
};
use temper_core::types::AuthenticatedProfile;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::{
    access_service, machine_client_service, machine_registration_service,
};
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;

/// Query flags for `GET /api/machine-clients`.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub include_revoked: bool,
}

/// Auth before writes: reject a non-admin before any mutation runs.
async fn require_admin(state: &AppState, authed: &AuthenticatedProfile) -> ApiResult<ProfileId> {
    let caller = ProfileId::from(authed.profile.id);
    if !access_service::is_system_admin(&state.pool, caller).await? {
        return Err(ApiError::Forbidden);
    }
    Ok(caller)
}

pub async fn provision(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ProvisionMachineRequest>,
) -> ApiResult<Json<MachineClient>> {
    let caller = require_admin(&state, &auth.0).await?;
    let client = machine_registration_service::provision(&state.pool, caller, &body).await?;
    Ok(Json(client))
}

pub async fn rebind(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(mut body): Json<RebindMachineRequest>,
) -> ApiResult<Json<MachineClient>> {
    let caller = require_admin(&state, &auth.0).await?;
    // The path segment is authoritative for which client is being rotated away from.
    body.from_machine_client_id = id;
    let client = machine_registration_service::rebind(&state.pool, caller, &body).await?;
    Ok(Json(client))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<MachineClient>>> {
    require_admin(&state, &auth.0).await?;
    Ok(Json(
        machine_client_service::list(&state.pool, q.include_revoked).await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MachineClient>> {
    require_admin(&state, &auth.0).await?;
    Ok(Json(machine_client_service::get(&state.pool, id).await?))
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MachineClient>> {
    let caller = require_admin(&state, &auth.0).await?;
    Ok(Json(
        machine_client_service::revoke(&state.pool, id, caller).await?,
    ))
}

pub async fn issue(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<IssueMachineRequest>,
) -> ApiResult<Json<IssuedMachineCredential>> {
    let caller = require_admin(&state, &auth.0).await?;
    let cred = machine_registration_service::issue(&state.pool, caller, &body).await?;
    Ok(Json(cred))
}

pub async fn rotate_secret(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RotateSecretRequest>,
) -> ApiResult<Json<IssuedMachineCredential>> {
    require_admin(&state, &auth.0).await?;
    let cred = machine_client_service::rotate_secret(&state.pool, id, body.grace_seconds).await?;
    Ok(Json(cred))
}
