//! Machine-client registration (G3 Phase A/B1/B2), documented under the `Machine Clients` tag.
//!
//! **Authorization lives in the services, not here** (Phase B2) — the same shape
//! `team_service` and `access_service` already use. It is `is_system_admin OR owner of the
//! machine's owning team` — `machine_authz::authorize` for provision and issue,
//! `authz::MachineClientControlAuthority` for acts on an existing machine, whose refusal is a `404`
//! indistinguishable from a missing id — and it is load-bearing rather than
//! defense-in-depth: since D11, `has_system_access` reads `kb_principal_standing` (approved), so
//! `require_system_access` on the gated router denies unapproved profiles, but the service-side
//! check is still the only real authorization (Phase A D12). The one exception is `rebind`, which
//! mints the `&SystemAdmin` proof itself and is mounted by the admin group (`routes/admin.rs`).
//!
//! The operation descriptions are the public contract and say only what a client needs: the gate,
//! the failure modes, and that a minted secret is shown once. The rationale stays in these doc
//! comments, which `summary`/`description` keep out of `openapi.json`.

use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use temper_core::types::machine::{
    IssueMachineRequest, IssuedMachineCredential, MachineClient, ProvisionMachineRequest,
    RebindMachineRequest, RotateSecretRequest,
};
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::{machine_client_service, machine_registration_service};
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;
use temper_core::types::query_params::MachineClientListQuery as ListQuery;

#[utoipa::path(
    post,
    operation_id = "provision_machine_client",
    summary = "Register a machine client",
    description = "Registers an externally issued IdP `client_id` as a machine principal and creates its agent profile, enrolling it in the listed teams and cogmap grants. Requires a system admin or the owner of `owner_team_id`; a machine with no owning team can only be registered by a system admin. A team owner can confer only reach they could grant themselves, and no caller can give a machine a team role above `member`.",
    path = "/api/machine-clients",
    tag = "Machine Clients",
    request_body = ProvisionMachineRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The registered machine client", body = MachineClient),
        (status = 400, description = "Unknown team role in `teams`", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is neither a system admin nor the owner of the owning team, a team role above `member` was requested (refused for every caller), a team owner requested reach they may not confer, or the caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 409, description = "The `client_id` is already registered", body = ErrorBody),
    )
)]
pub async fn provision(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ProvisionMachineRequest>,
) -> ApiResult<Json<MachineClient>> {
    let client = machine_registration_service::provision(&state.pool, &auth.0, &body).await?;
    Ok(Json(client))
}

/// POST /api/machine-clients/{id}/rebind — point a fresh IdP `client_id` at the agent profile an
/// existing machine client holds (system admin only). Mounted by `routes/admin.rs`, apart from its
/// owner-gated siblings: team ownership cannot bound the reach a rebind inherits.
#[utoipa::path(
    post,
    operation_id = "admin_rebind_machine_client",
    summary = "Rebind a machine client to a new client ID",
    description = "Points a fresh IdP `client_id` at the agent profile an existing machine client holds; by default the old client is revoked in the same transaction. The path `{id}` names the source client. Requires a system admin.",
    path = "/api/machine-clients/{id}/rebind",
    tag = "Admin",
    params(("id" = Uuid, Path, description = "The machine client whose profile the new client id inherits")),
    request_body = RebindMachineRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The new machine client, bound to the inherited profile", body = MachineClient),
        (status = 400, description = "The source client cannot be rebound (e.g. already revoked)", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such machine client", body = ErrorBody),
        (status = 409, description = "The new `client_id` is already registered", body = ErrorBody),
    )
)]
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

#[utoipa::path(
    get,
    operation_id = "list_machine_clients",
    summary = "List machine clients",
    description = "Lists the machine clients the caller may manage, newest first. A system admin sees every machine client; any other caller sees only those owned by a team they own. Revoked clients are omitted unless `include_revoked` is set.",
    path = "/api/machine-clients",
    tag = "Machine Clients",
    params(ListQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The machine clients visible to the caller", body = Vec<MachineClient>),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<MachineClient>>> {
    Ok(Json(
        machine_client_service::list(&state.pool, &auth.0, q.include_revoked).await?,
    ))
}

#[utoipa::path(
    get,
    operation_id = "get_machine_client",
    summary = "Get a machine client",
    description = "Returns one machine client. Requires a system admin or the owner of the machine's owning team. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/machine-clients/{id}",
    tag = "Machine Clients",
    params(("id" = Uuid, Path, description = "Machine client ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The machine client", body = MachineClient),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such machine client, or one the caller may not act on: the two are answered identically", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MachineClient>> {
    Ok(Json(
        machine_client_service::get_for_caller(&state.pool, &auth.0, id).await?,
    ))
}

#[utoipa::path(
    delete,
    operation_id = "revoke_machine_client",
    summary = "Revoke a machine client",
    description = "Revokes a machine client so its credential no longer authenticates, and revokes the agent profile's standing if it was approved. Team memberships and grants are left in place. Revoking an already-revoked client returns it unchanged. Requires a system admin or the owner of the machine's owning team. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/machine-clients/{id}",
    tag = "Machine Clients",
    params(("id" = Uuid, Path, description = "Machine client ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The revoked machine client", body = MachineClient),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such machine client, or one the caller may not act on: the two are answered identically", body = ErrorBody),
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MachineClient>> {
    Ok(Json(
        machine_client_service::revoke(&state.pool, id, &auth.0).await?,
    ))
}

#[utoipa::path(
    post,
    operation_id = "issue_machine_credential",
    summary = "Issue a machine credential",
    description = "Mints a new machine principal with a temper-issued `client_id` and secret. The `client_secret` in the response is shown once and never stored; only its hash is kept, so it cannot be retrieved later. Requires a system admin or the owner of `owner_team_id`; a machine with no owning team can only be issued by a system admin. Reach rules are those of registration.",
    path = "/api/machine-clients/issue",
    tag = "Machine Clients",
    request_body = IssueMachineRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The new machine client and its one-time `client_secret`", body = IssuedMachineCredential),
        (status = 400, description = "Unknown team role in `teams`", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is neither a system admin nor the owner of the owning team, a team role above `member` was requested (refused for every caller), a team owner requested reach they may not confer, or the caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
    )
)]
pub async fn issue(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<IssueMachineRequest>,
) -> ApiResult<Json<IssuedMachineCredential>> {
    let cred = machine_registration_service::issue(&state.pool, &auth.0, &body).await?;
    Ok(Json(cred))
}

#[utoipa::path(
    post,
    operation_id = "rotate_machine_client_secret",
    summary = "Rotate a machine client secret",
    description = "Installs a fresh secret for a temper-issued machine client. The previous secret stays valid for `grace_seconds` (0 to 604800). The new `client_secret` in the response is shown once and never stored. Requires a system admin or the owner of the machine's owning team. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/machine-clients/{id}/rotate-secret",
    tag = "Machine Clients",
    params(("id" = Uuid, Path, description = "Machine client ID")),
    request_body = RotateSecretRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The machine client and its new one-time `client_secret`", body = IssuedMachineCredential),
        (status = 400, description = "`grace_seconds` is out of range, the client was not issued by temper, or the client is revoked", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such machine client, or one the caller may not act on: the two are answered identically", body = ErrorBody),
    )
)]
pub async fn rotate_secret(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RotateSecretRequest>,
) -> ApiResult<Json<IssuedMachineCredential>> {
    let cred =
        machine_client_service::rotate_secret(&state.pool, &auth.0, id, body.grace_seconds).await?;
    Ok(Json(cred))
}
