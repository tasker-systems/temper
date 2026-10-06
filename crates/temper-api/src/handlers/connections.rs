//! Connections — temper's authed link to a remote system (S1 of "external systems as subscribed
//! emitters"), documented under the `Connections` tag. Not admin-only: a team owner manages the
//! connections their team owns, so the family stays in `gated_routes`, apart from the admin group.
//!
//! **Authorization lives in the service, not here** — `connection_service` gates provisioning on
//! `machine_authz::authorize` and every act on an existing connection on
//! `authz::ConnectionControlAuthority` (a system admin, or the OWNER of the connection's owning
//! team; teamless fails closed). A caller without authority over an existing connection is
//! answered `404`, exactly as for a missing id. As with machine clients, that check is
//! load-bearing rather than defense-in-depth: since D11, `has_system_access` reads
//! `kb_principal_standing`
//! (approved), so `require_system_access` on the gated router denies unapproved
//! profiles — but the service-side check is still the real authorization.

use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use temper_core::types::connection::{
    AttachCredentialResponse, Connection, ConnectionCredential, GrantConnectionReachRequest,
    ProvisionConnectionRequest, SetToolManifestRequest, SetWebhookEventsRequest,
};
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::connection_service;
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;
use temper_core::types::query_params::ConnectionListQuery as ListQuery;

#[utoipa::path(
    post,
    operation_id = "provision_connection",
    summary = "Provision a connection",
    description = "Provisions a connection to a remote system, with its own agent profile, emitter and home context. It starts with no credential and with no webhook events or tool manifest; each is attached by its own call. Requires a system admin or the owner of `owner_team_id`; a connection with no owning team can only be provisioned by a system admin. Owning a connection does not grant any team read-reach on it.",
    path = "/api/connections",
    tag = "Connections",
    request_body = ProvisionConnectionRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The provisioned connection", body = Connection),
        (status = 400, description = "`provider` or `name` is empty, or `name` has no characters usable in a slug", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is neither a system admin nor the owner of the owning team, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 409, description = "No free slug could be derived from `name`", body = ErrorBody),
    )
)]
pub async fn provision(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ProvisionConnectionRequest>,
) -> ApiResult<Json<Connection>> {
    let connection = connection_service::provision(&state.pool, &auth.0, &body).await?;
    Ok(Json(connection))
}

#[utoipa::path(
    get,
    operation_id = "list_connections",
    summary = "List connections",
    description = "Lists the connections the caller may manage, newest first. A system admin sees every connection; any other caller sees only those owned by a team they own. Revoked connections are omitted unless `include_revoked` is set.",
    path = "/api/connections",
    tag = "Connections",
    params(ListQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The connections visible to the caller", body = Vec<Connection>),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<Connection>>> {
    Ok(Json(
        connection_service::list(&state.pool, &auth.0, q.include_revoked).await?,
    ))
}

#[utoipa::path(
    get,
    operation_id = "get_connection",
    summary = "Get a connection",
    description = "Returns one connection. Requires a system admin or the owner of the connection's owning team. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/connections/{id}",
    tag = "Connections",
    params(("id" = Uuid, Path, description = "Connection ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The connection", body = Connection),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such connection, or one the caller may not act on: the two are answered identically", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Connection>> {
    Ok(Json(
        connection_service::get_for_caller(&state.pool, &auth.0, id).await?,
    ))
}

#[utoipa::path(
    delete,
    operation_id = "revoke_connection",
    summary = "Revoke a connection",
    description = "Revokes a connection so temper mints no new tokens for it. Tokens already minted stay valid at the remote system until they expire. The connection's profile, emitter and history are kept. Revoking an already-revoked connection returns it unchanged. Requires a system admin or the owner of the connection's owning team. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/connections/{id}",
    tag = "Connections",
    params(("id" = Uuid, Path, description = "Connection ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The revoked connection", body = Connection),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such connection, or one the caller may not act on: the two are answered identically", body = ErrorBody),
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Connection>> {
    Ok(Json(
        connection_service::revoke(&state.pool, id, &auth.0).await?,
    ))
}

/// Attach the credential — what flips `needs_credential` off. The body carries no secret: it names
/// a broker and a connector that broker holds the secret for.
#[utoipa::path(
    post,
    operation_id = "attach_connection_credential",
    summary = "Attach a connection credential",
    description = "Attaches the credential reference: a broker and a connector the broker holds the secret for. The body carries no secret. temper mints once to verify the connector and reports what it observed; a connector the broker rejects fails the request, while pending consent or an unconfigured broker is reported in `verification.note`. Requires a system admin or the owner of the connection's owning team. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/connections/{id}/credential",
    tag = "Connections",
    params(("id" = Uuid, Path, description = "Connection ID")),
    request_body = ConnectionCredential,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The updated connection and the verification result", body = AttachCredentialResponse),
        (status = 400, description = "`broker` or `connector` is empty, or the broker rejected the connector", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such connection, or one the caller may not act on: the two are answered identically", body = ErrorBody),
        (status = 409, description = "The connection is revoked", body = ErrorBody),
    )
)]
pub async fn attach_credential(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ConnectionCredential>,
) -> ApiResult<Json<AttachCredentialResponse>> {
    Ok(Json(
        connection_service::attach_credential(
            &state.pool,
            state.broker.as_ref(),
            &auth.0,
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
#[utoipa::path(
    post,
    operation_id = "set_connection_webhook_events",
    summary = "Set connection webhook events",
    description = "Replaces the set of remote event types the connection receives. A non-empty set makes the connection ledger-capable. Requires a system admin or the owner of the connection's owning team. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/connections/{id}/webhook-events",
    tag = "Connections",
    params(("id" = Uuid, Path, description = "Connection ID")),
    request_body = SetWebhookEventsRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The updated connection", body = Connection),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such connection, or one the caller may not act on: the two are answered identically", body = ErrorBody),
        (status = 409, description = "The connection is revoked", body = ErrorBody),
    )
)]
pub async fn set_webhook_events(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetWebhookEventsRequest>,
) -> ApiResult<Json<Connection>> {
    Ok(Json(
        connection_service::set_webhook_events(&state.pool, &auth.0, id, &body.events).await?,
    ))
}

/// Declare the read-only remote tools. Non-empty ⇒ reach-capable.
#[utoipa::path(
    post,
    operation_id = "set_connection_tool_manifest",
    summary = "Set connection tool manifest",
    description = "Replaces the declared read-only remote tools. A non-empty manifest makes the connection reach-capable. Requires a system admin or the owner of the connection's owning team. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/connections/{id}/tool-manifest",
    tag = "Connections",
    params(("id" = Uuid, Path, description = "Connection ID")),
    request_body = SetToolManifestRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The updated connection", body = Connection),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such connection, or one the caller may not act on: the two are answered identically", body = ErrorBody),
        (status = 409, description = "The connection is revoked", body = ErrorBody),
    )
)]
pub async fn set_tool_manifest(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SetToolManifestRequest>,
) -> ApiResult<Json<Connection>> {
    Ok(Json(
        connection_service::set_tool_manifest(&state.pool, &auth.0, id, &body.tools).await?,
    ))
}

/// Grant a TEAM read-reach on this connection. Owning a connection is not reaching it — this writes
/// a `kb_access_grants` row so the named team's members inherit read on what the connection receives.
#[utoipa::path(
    post,
    operation_id = "grant_connection_reach",
    summary = "Grant a team read-reach on a connection",
    description = "Lets the members of `team` read what the connection receives. Reach is read-only. Requires a system admin, or the owner of the connection's owning team who also owns or maintains the receiving team. When the connection declares a remote reach the attach-time verification did not confirm, `affirm_reach` must state why the binding is intended; it is refused when there is nothing to affirm. A caller with system access who does not control the connection is answered 404, exactly as for an id that does not exist.",
    path = "/api/connections/{id}/reach",
    tag = "Connections",
    params(("id" = Uuid, Path, description = "Connection ID")),
    request_body = GrantConnectionReachRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The connection, with any affirmation recorded", body = Connection),
        (status = 400, description = "`affirm_reach` was given but the connection has no reach gap to acknowledge", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "The caller controls the connection but does not own or maintain the receiving team, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such connection, or one the caller does not control (answered identically); or, for a system admin, no such receiving team", body = ErrorBody),
        (status = 409, description = "The connection declares a remote reach that must be affirmed; resend with `affirm_reach`", body = ErrorBody),
    )
)]
pub async fn grant_reach(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<GrantConnectionReachRequest>,
) -> ApiResult<Json<Connection>> {
    Ok(Json(
        connection_service::grant_reach(&state.pool, &auth.0, id, body.team, body.affirm_reach)
            .await?,
    ))
}

/// Revoke a team's read-reach on this connection. Idempotent — an absent grant is a no-op.
#[utoipa::path(
    delete,
    operation_id = "revoke_connection_reach",
    summary = "Revoke a team's read-reach on a connection",
    description = "Removes the read-reach grant for `team`. Revoking an absent grant is a no-op. `affirm_reach` is ignored. Requires a system admin or the owner of the connection's owning team; no role on the receiving team is needed. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/connections/{id}/reach",
    tag = "Connections",
    params(("id" = Uuid, Path, description = "Connection ID")),
    request_body = GrantConnectionReachRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The connection", body = Connection),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such connection, or one the caller may not act on: the two are answered identically", body = ErrorBody),
    )
)]
pub async fn revoke_reach(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<GrantConnectionReachRequest>,
) -> ApiResult<Json<Connection>> {
    Ok(Json(
        connection_service::revoke_reach(&state.pool, &auth.0, id, body.team).await?,
    ))
}
