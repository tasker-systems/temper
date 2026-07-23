//! Machine-client registration (G3 Phase A/B1/B2). Out of the OpenAPI contract (plain
//! `.route()` mounting), like `/api/access/admin/*`.
//!
//! **Authorization lives in the services, not here** (Phase B2) — the same shape
//! `team_service` and `access_service` already use. It is `is_system_admin OR owner of the
//! machine's owning team`, and it is load-bearing rather than defense-in-depth: production
//! runs `access_mode = 'open'`, under which `has_system_access` is true for every profile, so
//! `require_system_access` on the gated router admits everyone. The service-side check is the
//! only thing protecting these endpoints (Phase A D12).

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{
    IssueMachineRequest, IssuedMachineCredential, MachineClient, ProvisionMachineRequest,
    RebindMachineRequest, RotateSecretRequest,
};
use temper_services::error::ApiResult;
use temper_services::services::{machine_client_service, machine_registration_service};
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;

/// Query flags for `GET /api/machine-clients`.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub include_revoked: bool,
}

pub async fn provision(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ProvisionMachineRequest>,
) -> ApiResult<Json<MachineClient>> {
    let caller = ProfileId::from(auth.0.profile().id);
    let client = machine_registration_service::provision(&state.pool, caller, &body).await?;
    Ok(Json(client))
}

pub async fn rebind(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(mut body): Json<RebindMachineRequest>,
) -> ApiResult<Json<MachineClient>> {
    // rebind is system-admin-only (B2): the &SystemAdmin proof is the gate (admin-authz enclosure,
    // spec §3), minted here. Team ownership cannot bound the reach a rebind inherits.
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    // The path segment is authoritative for which client is being rotated away from.
    body.from_machine_client_id = id;
    let client = machine_registration_service::rebind(&state.pool, &admin, &body).await?;
    Ok(Json(client))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<MachineClient>>> {
    let caller = ProfileId::from(auth.0.profile().id);
    Ok(Json(
        machine_client_service::list(&state.pool, caller, q.include_revoked).await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MachineClient>> {
    let caller = ProfileId::from(auth.0.profile().id);
    Ok(Json(
        machine_client_service::get_for_caller(&state.pool, caller, id).await?,
    ))
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MachineClient>> {
    let caller = ProfileId::from(auth.0.profile().id);
    Ok(Json(
        machine_client_service::revoke(&state.pool, id, caller).await?,
    ))
}

pub async fn issue(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<IssueMachineRequest>,
) -> ApiResult<Json<IssuedMachineCredential>> {
    let caller = ProfileId::from(auth.0.profile().id);
    let cred = machine_registration_service::issue(&state.pool, caller, &body).await?;
    Ok(Json(cred))
}

pub async fn rotate_secret(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RotateSecretRequest>,
) -> ApiResult<Json<IssuedMachineCredential>> {
    let caller = ProfileId::from(auth.0.profile().id);
    let cred =
        machine_client_service::rotate_secret(&state.pool, caller, id, body.grace_seconds).await?;
    Ok(Json(cred))
}
