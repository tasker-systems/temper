//! Connections — temper's authed link to a remote system (S1 of "external systems as subscribed
//! emitters"). Out of the OpenAPI contract (plain `.route()` mounting), like
//! `/api/machine-clients` and `/api/access/admin/*`: this is an admin surface, not a public one.
//!
//! **Authorization lives in the service, not here** — `connection_service` calls
//! `machine_authz::authorize` (a system admin, or the OWNER of the connection's owning team;
//! teamless fails closed). As with machine clients, that check is load-bearing rather than
//! defense-in-depth: production runs `access_mode = 'open'`, under which `has_system_access` is
//! true for every profile, so `require_system_access` on the gated router admits everyone.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::connection::{Connection, ProvisionConnectionRequest};
use temper_core::types::ids::ProfileId;
use temper_services::error::ApiResult;
use temper_services::services::connection_service;
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;

/// Query flags for `GET /api/connections`.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub include_revoked: bool,
}

pub async fn provision(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ProvisionConnectionRequest>,
) -> ApiResult<Json<Connection>> {
    let caller = ProfileId::from(auth.0.profile.id);
    let connection = connection_service::provision(&state.pool, caller, &body).await?;
    Ok(Json(connection))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<Connection>>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        connection_service::list(&state.pool, caller, q.include_revoked).await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Connection>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        connection_service::get_for_caller(&state.pool, caller, id).await?,
    ))
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Connection>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        connection_service::revoke(&state.pool, id, caller).await?,
    ))
}
