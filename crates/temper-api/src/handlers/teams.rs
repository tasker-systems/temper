//! Team lifecycle handlers — thin: extract `AuthUser` (`AnyPrincipal` for the two reads a machine
//! may take), dispatch one `team_service` call, return the typed row. Service-direct (no Backend-trait
//! command, no event emission) per org-provisioning spec §2.6.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::{AnyPrincipal, AuthUser};
use temper_core::types::ids::ProfileId;
use temper_core::types::reassign::RemoveMemberOutcome;
use temper_core::types::team::{
    AddMemberRequest, AttachChildOutcome, ChangeRoleRequest, PublicWithinEffects,
    PublicWithinOutcome, PublicWithinPolicyOutcome, PublicWithinPolicyRequest, PublicWithinRowRef,
    RenameTeamSlugOutcome, RenameTeamSlugRequest, ReparentOutcome, ReparentRequest,
    RestoreTeamOutcome, SetAutoJoinOutcome, SetAutoJoinRequest, TeamActAnswerQuery,
    TeamCreateRequest, TeamDetail, TeamMemberRow, TeamRow, TeamUpdateRequest,
};
use temper_services::error::ApiResult;
use temper_services::services::team_service;
use temper_services::state::AppState;

/// List teams you belong to
#[utoipa::path(
    get,
    operation_id = "list_teams",
    path = "/api/teams",
    tag = "Teams",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Teams the caller is a member of", body = Vec<TeamRow>),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AnyPrincipal,
) -> ApiResult<Json<Vec<TeamRow>>> {
    team_service::list_teams(&state.pool, ProfileId::from(auth.0.profile().id))
        .await
        .map(Json)
}

/// Create a team
///
/// With `?answer=effects` the row also carries `public_within` (what the new team's parent links did
/// to `public_within` rows) and `public_within_in_reach` (the effective rows the new team's roster
/// reads through its parents), each row named only to a caller who manages its team or anchor, or is
/// a system admin, and counted otherwise. Without it the answer is the row alone.
#[utoipa::path(
    post,
    operation_id = "create_team",
    path = "/api/teams",
    tag = "Teams",
    params(TeamActAnswerQuery),
    security(("bearer_auth" = [])),
    request_body = TeamCreateRequest,
    responses(
        (status = 201, description = "Team created (with `?answer=effects`, carrying `public_within` and `public_within_in_reach`)", body = TeamRow),
        (status = 400, description = "The `personal-` slug prefix is reserved for personal teams; the parent is a personal team, which takes no children; `auto_join_role` is above `member` (it may be `member` or `watcher`: a role that governs a team is granted by a person); or the caller is a machine principal (a team's creator becomes its owner, and a machine never governs)"),
        (status = 403, description = "Forbidden (a child requires owner/maintainer on the parent, and a system admin when the parent is temper-system or the gating team; auto_join_role requires admin); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 409, description = "Team slug already exists"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(answer): Query<TeamActAnswerQuery>,
    Json(body): Json<TeamCreateRequest>,
) -> ApiResult<(StatusCode, Json<TeamRow>)> {
    if !answer.wants_effects() {
        let row = team_service::create_team(&state.pool, &auth.0, &body).await?;
        return Ok((StatusCode::CREATED, Json(row)));
    }
    let created = team_service::create_team_with_effects(&state.pool, &auth.0, &body).await?;
    let mut row = created.team;
    row.public_within = Some(created.public_within);
    row.public_within_in_reach = Some(created.public_within_in_reach);
    Ok((StatusCode::CREATED, Json(row)))
}

/// Add a member to a team
#[utoipa::path(
    post,
    path = "/api/teams/{id}/members",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    request_body = AddMemberRequest,
    responses(
        (status = 201, description = "Member added", body = TeamMemberRow),
        (status = 403, description = "Forbidden: the caller is not an owner or maintainer of this team (on the gating team or temper-system, not a system admin; on a personal team, not its owner — a system admin included); or (`FORBIDDEN_DETAIL`) the role is above the caller's own — a manager grants up to their own role, a system admin any; or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 400, description = "The target is a machine principal and the role is above `member`; or the team is a personal team and the target is neither its owner nor one of the owner's machines"),
        (status = 404, description = "Team not found (told only to a system admin; any other caller gets 403)"),
        (status = 409, description = "Already a member of this team; change the role with PATCH /api/teams/{id}/members/{profile_id}"),
    )
)]
pub async fn add_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<AddMemberRequest>,
) -> ApiResult<(StatusCode, Json<TeamMemberRow>)> {
    let row = team_service::add_member(&state.pool, &auth.0, team_id, &body).await?;
    Ok((StatusCode::CREATED, Json(row)))
}

/// Get one team
#[utoipa::path(
    get,
    path = "/api/teams/{id}",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Team detail + members", body = TeamDetail),
        (status = 404, description = "Team not found or not visible to caller"),
    )
)]
pub async fn detail(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(team_id): Path<Uuid>,
) -> ApiResult<Json<TeamDetail>> {
    team_service::team_detail(&state.pool, &auth.0, team_id)
        .await
        .map(Json)
}

/// Update a team
#[utoipa::path(
    patch,
    operation_id = "update_team",
    path = "/api/teams/{id}",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    request_body = TeamUpdateRequest,
    responses(
        (status = 200, description = "Team metadata updated", body = TeamRow),
        (status = 403, description = "Forbidden (caller is not owner/maintainer; on temper-system or the gating team, not a system admin); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "Team not found or soft-deleted"),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<TeamUpdateRequest>,
) -> ApiResult<Json<TeamRow>> {
    team_service::update_team(&state.pool, &auth.0, team_id, &body)
        .await
        .map(Json)
}

/// Delete a team
///
/// With `?answer=effects` the answer is `200` with what the delete did to `public_within` rows (rows
/// over or under the team stop conferring), each named only to a caller who manages its team or
/// anchor, or is a system admin, and counted otherwise. Without it the answer is `204`.
#[utoipa::path(
    delete,
    operation_id = "delete_team",
    path = "/api/teams/{id}",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID"), TeamActAnswerQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Team soft-deleted; asked with `?answer=effects`", body = PublicWithinEffects),
        (status = 204, description = "Team soft-deleted"),
        (status = 403, description = "Forbidden (caller is neither the owner nor a system admin; on temper-system or the gating team, not a system admin; on a personal team, not its owner, a system admin included); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "Team not found or already soft-deleted"),
        (status = 409, description = "Cannot delete the temper-system root team"),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Query(answer): Query<TeamActAnswerQuery>,
) -> ApiResult<Response> {
    let effects = team_service::delete_team(&state.pool, &auth.0, team_id).await?;
    Ok(effects_or_no_content(answer, effects))
}

/// The answer of an act whose default is `204`: `200` with its `public_within` effects when the
/// caller asked for them, `204` otherwise.
fn effects_or_no_content(answer: TeamActAnswerQuery, effects: PublicWithinEffects) -> Response {
    if answer.wants_effects() {
        (StatusCode::OK, Json(effects)).into_response()
    } else {
        StatusCode::NO_CONTENT.into_response()
    }
}

/// Remove a member from a team
#[utoipa::path(
    delete,
    path = "/api/teams/{id}/members/{profile_id}",
    tag = "Teams",
    params(
        ("id" = Uuid, Path, description = "Team ID"),
        ("profile_id" = Uuid, Path, description = "Member profile ID"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Member removed; residual owned-resource reach reported. A person's machines on the team (ones they registered, rotated, or whose owning team they own) are removed with them, unless the team owns the machine or another person still on it controls it; each is recorded under the same correlation id", body = RemoveMemberOutcome),
        (status = 403, description = "Forbidden: the caller is neither the member leaving nor an owner or maintainer of this team (on the gating team or temper-system, not a system admin; on a personal team, not its owner — a system admin included); or (`FORBIDDEN_DETAIL`) the member is an owner other than the caller, or ranks above the caller — only a system admin removes another owner; or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "Member not found, or (to a system admin) the team does not exist; told only to a caller who may manage the team"),
        (status = 409, description = "Cannot remove the last owner (grant `owner` to another member first), a SAML-provisioned row, or a profile from its own personal team"),
    )
)]
pub async fn remove_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, profile_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<RemoveMemberOutcome>> {
    let outcome = team_service::remove_member(&state.pool, &auth.0, team_id, profile_id).await?;
    Ok(Json(outcome))
}

/// Change a member's role
#[utoipa::path(
    patch,
    path = "/api/teams/{id}/members/{profile_id}",
    tag = "Teams",
    params(
        ("id" = Uuid, Path, description = "Team ID"),
        ("profile_id" = Uuid, Path, description = "Member profile ID"),
    ),
    security(("bearer_auth" = [])),
    request_body = ChangeRoleRequest,
    responses(
        (status = 200, description = "Role changed", body = TeamMemberRow),
        (status = 400, description = "The target is a machine principal and the role is above `member`"),
        (status = 403, description = "Forbidden: the caller is not an owner or maintainer of this team (on the gating team or temper-system, not a system admin; on a personal team, not its owner — a system admin included); or (`FORBIDDEN_DETAIL`) the role is above the caller's own, the member is an owner other than the caller or ranks above the caller (only a system admin changes another owner), or the caller is raising their own role; or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "Member not found, or (to a system admin) the team does not exist; told only to a caller who may manage the team"),
        (status = 409, description = "Cannot demote the last owner (grant `owner` to another member first), or a SAML-provisioned row; or the membership changed since it was read (retry)"),
    )
)]
pub async fn change_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, profile_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<ChangeRoleRequest>,
) -> ApiResult<Json<TeamMemberRow>> {
    team_service::change_role(&state.pool, &auth.0, team_id, profile_id, body.role)
        .await
        .map(Json)
}

/// Detach a child team from this team
///
/// Removes the parent link, so the child's members stop reaching what is attached to this team and
/// its ancestors. The child and its roster survive; only this link is cut. A manager of the parent
/// or of the child may detach (a child's managers may leave any parent). Recorded on the admin
/// ledger as `team_parent_link_changed`. With `?answer=effects` the answer is `200` with what the
/// detach did to `public_within` rows (a row whose anchor is no longer above its team by any path is
/// deleted, `lapsed`), each named only to a caller who manages its team or anchor, or is a system
/// admin, and counted otherwise. Without it the answer is `204`.
#[utoipa::path(
    delete,
    operation_id = "detach_child_team",
    path = "/api/teams/{id}/children/{child_id}",
    tag = "Teams",
    params(
        ("id" = Uuid, Path, description = "Parent team ID"),
        ("child_id" = Uuid, Path, description = "Child team ID"),
        TeamActAnswerQuery,
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Child detached; asked with `?answer=effects`", body = PublicWithinEffects),
        (status = 204, description = "Child detached"),
        (status = 403, description = "Forbidden: the caller manages neither the child team nor the parent team (neither arm acts on temper-system or the gating team) and is not a system admin, or the child is a personal team (nobody detaches one); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "That team is not a child of this team; told only to a caller who may detach it"),
    )
)]
pub async fn detach_child(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, child_id)): Path<(Uuid, Uuid)>,
    Query(answer): Query<TeamActAnswerQuery>,
) -> ApiResult<Response> {
    let effects = team_service::detach_child(&state.pool, &auth.0, team_id, child_id).await?;
    Ok(effects_or_no_content(answer, effects))
}

/// Attach a child team under this team
///
/// Writes a parent link: every member of the child, at any role, then reaches everything attached
/// to this team and its ancestors. Also the door for a second parent. The caller must manage both
/// teams (owner or maintainer of each); attaching under temper-system or the gating team, or
/// attaching an auto-join team as the child, is a system admin's act. Recorded on the admin ledger
/// as `team_parent_link_changed`; a link that already exists answers `exists` and records nothing.
#[utoipa::path(
    put,
    operation_id = "attach_child_team",
    path = "/api/teams/{id}/children/{child_id}",
    tag = "Teams",
    params(
        ("id" = Uuid, Path, description = "Parent team ID"),
        ("child_id" = Uuid, Path, description = "Child team ID"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Linked, or the link already existed", body = AttachChildOutcome),
        (status = 400, description = "A personal team on either side, temper-system as the child, or the team as its own parent"),
        (status = 403, description = "Forbidden: the caller does not manage both teams, or the parent or the child is temper-system or the gating team, or the child is an auto-join team, and the caller is not a system admin; or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "Either team does not exist or is soft-deleted; told only to a caller who may manage both"),
        (status = 409, description = "The link would make a team its own ancestor"),
    )
)]
pub async fn attach_child(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, child_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<AttachChildOutcome>> {
    team_service::attach_child(&state.pool, &auth.0, team_id, child_id)
        .await
        .map(Json)
}

/// Move this team from one parent to another
///
/// One act: the new link is written before the old one is cut, so a refusal leaves the team where
/// it was and it never holds both reaches. The caller must manage this team and the new parent;
/// a new parent that is temper-system or the gating team, or moving an auto-join team, is a system
/// admin's act. Both `team_parent_link_changed` records carry the returned `correlation_id`.
#[utoipa::path(
    post,
    operation_id = "reparent_team",
    path = "/api/teams/{id}/reparent",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID (the child being moved)")),
    security(("bearer_auth" = [])),
    request_body = ReparentRequest,
    responses(
        (status = 200, description = "Moved", body = ReparentOutcome),
        (status = 400, description = "`from` and `to` are the same team, or a structural refusal (a personal team, temper-system as the child)"),
        (status = 403, description = "Forbidden: the caller does not manage this team, or does not manage `to` (an unknown `to` included), or `to` or this team is temper-system or the gating team, or this team is an auto-join team, and the caller is not a system admin; or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "`from` does not resolve (`to` only to a system admin), a team is soft-deleted, or `from` is not a parent of this team; told only to a caller who manages this team"),
        (status = 409, description = "The move would make a team its own ancestor; nothing changed"),
    )
)]
pub async fn reparent(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<ReparentRequest>,
) -> ApiResult<Json<ReparentOutcome>> {
    team_service::reparent(&state.pool, &auth.0, team_id, &body)
        .await
        .map(Json)
}

/// Restore a soft-deleted team
///
/// The inverse of delete. Every preserved row confers again: the roster, the parent links, and the
/// children's reach through this team; the response lists all three so the restorer sees the reach
/// they revived. The team's owner, or a system admin (temper-system and the gating team: a system
/// admin only; a personal team: its owner only). Recorded as `team_lifecycle_changed`.
#[utoipa::path(
    post,
    operation_id = "restore_team",
    path = "/api/teams/{id}/restore",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Restored", body = RestoreTeamOutcome),
        (status = 403, description = "Forbidden: the caller is not the team's owner or a system admin (temper-system, the gating team or an auto-join team: not a system admin; a personal team: not its owner); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "The team does not exist or is not deleted"),
    )
)]
pub async fn restore(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
) -> ApiResult<Json<RestoreTeamOutcome>> {
    team_service::restore_team(&state.pool, &auth.0, team_id)
        .await
        .map(Json)
}

/// Set, change or clear a team's auto-join role
///
/// A system admin's act. With a role set, every approved profile (never a machine) is enrolled at
/// it, and so reads everything attached to the team and to each of the `ancestors` returned.
/// Existing memberships are never changed: not raised, not lowered, not removed. `null` stops
/// enrolment. Refused on temper-system, the gating team and personal teams. Recorded as
/// `team_lifecycle_changed`.
#[utoipa::path(
    put,
    operation_id = "set_team_auto_join",
    path = "/api/teams/{id}/auto-join",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    request_body = SetAutoJoinRequest,
    responses(
        (status = 200, description = "Set, changed, cleared, or already so", body = SetAutoJoinOutcome),
        (status = 400, description = "A role above `member`, or the team is temper-system, the gating team or a personal team"),
        (status = 403, description = "Caller is not a system admin, or lacks system access (`SYSTEM_ACCESS_REQUIRED`); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "The team does not exist or is soft-deleted"),
    )
)]
pub async fn set_auto_join(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<SetAutoJoinRequest>,
) -> ApiResult<Json<SetAutoJoinOutcome>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    team_service::set_auto_join(&state.pool, &admin, team_id, body.role)
        .await
        .map(Json)
}

/// Change a team's slug
///
/// Its own door, not a field of `PATCH`. **Addresses change:** `+old/<context>` refs stop resolving
/// (UUID refs are unaffected), and refs stored in content are not rewritten. An owner or maintainer;
/// temper-system and the gating team: a system admin (renaming the gating team carries the
/// instance setting with it). Recorded as `team_lifecycle_changed`.
#[utoipa::path(
    post,
    operation_id = "rename_team_slug",
    path = "/api/teams/{id}/rename-slug",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    request_body = RenameTeamSlugRequest,
    responses(
        (status = 200, description = "Renamed (or already so)", body = RenameTeamSlugOutcome),
        (status = 400, description = "The slug is invalid or starts with `personal-`, or the team is temper-system or a personal team"),
        (status = 403, description = "Forbidden: the caller is not an owner or maintainer of the team (temper-system or the gating team: not a system admin); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "The team does not exist or is soft-deleted"),
        (status = 409, description = "The slug is taken, by a live or a soft-deleted team"),
    )
)]
pub async fn rename_slug(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<RenameTeamSlugRequest>,
) -> ApiResult<Json<RenameTeamSlugOutcome>> {
    team_service::rename_slug(&state.pool, &auth.0, team_id, &body)
        .await
        .map(Json)
}

/// Open a team's contexts to an ancestor's subtree
///
/// `public_within(T, A)`: the people under the anchor (on its roster or on a live team beneath it,
/// at any role) read every context the team owns, and reference it from homes they author. Never a
/// write into the team, and never a machine or a connection agent. Not inherited by the team's
/// children. Both sides consent: the caller is a direct owner or maintainer of the team (a system
/// admin is held to this too), and of the anchor, or is a system admin. An anchor that is
/// temper-system or the gating team, or whose subtree holds an auto-join or the gating team, is a
/// system admin's act (`instance_wide`). The audience grows with the anchor's subtree: every manager
/// of a team under the anchor, identity-provider group sync and an admin can widen it. Either side
/// closes alone. Recorded as `team_public_within_changed`.
#[utoipa::path(
    put,
    operation_id = "open_public_within",
    path = "/api/teams/{id}/public-within/{anchor_id}",
    tag = "Teams",
    params(
        ("id" = Uuid, Path, description = "The team whose owned contexts are opened (T)"),
        ("anchor_id" = Uuid, Path, description = "The ancestor team whose subtree reads them (A)"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Opened, or already open", body = PublicWithinOutcome),
        (status = 400, description = "The anchor is not a live team above this one; or the team is a personal team, temper-system or the gating team, or the anchor is a personal team"),
        (status = 403, description = "Forbidden: the caller is not a direct owner or maintainer of the team, or of the anchor and not a system admin; or (`FORBIDDEN_DETAIL`) `public_within_forbidden_here`, a forbid applies over the team (temper-system's is the instance-wide off), or the anchor needs a system admin; or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
    )
)]
pub async fn open_public_within(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, anchor_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<PublicWithinOutcome>> {
    team_service::open_public_within(&state.pool, &auth.0, team_id, anchor_id)
        .await
        .map(Json)
}

/// Close a team's opening to an ancestor's subtree
///
/// Deletes the `public_within` row. Either side closes alone, whatever the policy or either team's
/// liveness: a direct owner or maintainer of the team, of the anchor (temper-system or the gating
/// team as the anchor: a system admin only), or a system admin. What readers already read is not
/// recalled. Recorded as `team_public_within_changed`.
#[utoipa::path(
    delete,
    operation_id = "close_public_within",
    path = "/api/teams/{id}/public-within/{anchor_id}",
    tag = "Teams",
    params(
        ("id" = Uuid, Path, description = "The team whose owned contexts are opened (T)"),
        ("anchor_id" = Uuid, Path, description = "The ancestor team whose subtree reads them (A)"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Closed"),
        (status = 403, description = "Forbidden: the caller manages neither side and is not a system admin (whether or not the row exists); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "No such row (told only to a caller on a side, or a system admin)"),
    )
)]
pub async fn close_public_within(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, anchor_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    team_service::close_public_within(&state.pool, &auth.0, team_id, anchor_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Forbid or allow public_within over a team's subtree
///
/// `forbidden: true` makes every `public_within` row over this team, and over every team beneath it
/// by any path, live or dead, confer nothing; a lower team's own setting never lifts it. On
/// temper-system it is the instance-wide off. Rows are kept and revive when the forbid is cleared.
/// The team's direct owner, or a system admin; temper-system and the gating team: a system admin
/// only. Recorded as `public_within_policy_changed`, with one `public_within_effect_changed` per row
/// whose effect it flips.
#[utoipa::path(
    put,
    operation_id = "set_public_within_policy",
    path = "/api/teams/{id}/public-within-policy",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    request_body = PublicWithinPolicyRequest,
    responses(
        (status = 200, description = "Set, cleared, or already so; the rows it made inert or revived", body = PublicWithinPolicyOutcome),
        (status = 400, description = "The team is a personal team"),
        (status = 403, description = "Forbidden: the caller is not the team's direct owner and not a system admin (temper-system or the gating team: not a system admin); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = temper_services::error::ErrorBody),
        (status = 404, description = "The team does not exist or is soft-deleted (told only to a system admin)"),
    )
)]
pub async fn set_public_within_policy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(body): Json<PublicWithinPolicyRequest>,
) -> ApiResult<Json<PublicWithinPolicyOutcome>> {
    team_service::set_public_within_policy(&state.pool, &auth.0, team_id, body.forbidden)
        .await
        .map(Json)
}

/// List the openings that confer on you
///
/// The `public_within` rows anchored on a team you are under that confer right now: each names the
/// team whose owned contexts you read through it, and the anchor. An inert row is absent.
#[utoipa::path(
    get,
    operation_id = "list_public_within",
    path = "/api/teams/public-within",
    tag = "Teams",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The rows that confer on the caller", body = Vec<PublicWithinRowRef>),
        (status = 403, description = "The caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal): a machine never reads through public_within", body = temper_services::error::ErrorBody),
    )
)]
pub async fn list_public_within(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<PublicWithinRowRef>>> {
    team_service::list_public_within(&state.pool, &auth.0)
        .await
        .map(Json)
}
