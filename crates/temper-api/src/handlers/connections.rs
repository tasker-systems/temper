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

use temper_core::types::connection::{
    AttachCredentialResponse, Connection, ConnectionCredential, GrantConnectionReachRequest,
    ProvisionConnectionRequest, SetToolManifestRequest, SetWebhookEventsRequest,
};
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

/// Attach the credential — what flips `needs_credential` off. The body carries no secret: it names
/// a broker and a connector that broker holds the secret for.
pub async fn attach_credential(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ConnectionCredential>,
) -> ApiResult<Json<AttachCredentialResponse>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        connection_service::attach_credential(
            &state.pool,
            state.broker.as_ref(),
            caller,
            id,
            &body,
        )
        .await?,
    ))
}

/// Register the remote event types. Non-empty ⇒ ledger-capable.
///
/// Its own endpoint rather than a field on a general update, because the two capability tiers are
/// separately provisioned and both explicit — collapsing them into one PATCH would let a caller
/// grant reach while believing they were only registering a webhook.
pub async fn set_webhook_events(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetWebhookEventsRequest>,
) -> ApiResult<Json<Connection>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        connection_service::set_webhook_events(&state.pool, caller, id, &body.events).await?,
    ))
}

/// Declare the read-only remote tools. Non-empty ⇒ reach-capable.
pub async fn set_tool_manifest(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetToolManifestRequest>,
) -> ApiResult<Json<Connection>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        connection_service::set_tool_manifest(&state.pool, caller, id, &body.tools).await?,
    ))
}

/// Grant a TEAM read-reach on this connection. Owning a connection is not reaching it — this writes
/// a `kb_access_grants` row so the named team's members inherit read on what the connection receives.
pub async fn grant_reach(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<GrantConnectionReachRequest>,
) -> ApiResult<Json<Connection>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        connection_service::grant_reach(&state.pool, caller, id, body.team, body.affirm_reach)
            .await?,
    ))
}

/// Revoke a team's read-reach on this connection. Idempotent — an absent grant is a no-op.
pub async fn revoke_reach(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<GrantConnectionReachRequest>,
) -> ApiResult<Json<Connection>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        connection_service::revoke_reach(&state.pool, caller, id, body.team).await?,
    ))
}
