//! Team membership commands: join, status, leave; plus the lifecycle surface
//! (create, add-member, list) that round-trips CLI → client → API → service.

use crate::actions::cogmap::resolve_team_id;
use crate::error::{Result, TemperError};
use crate::output;
use temper_core::types::team::{
    AddMemberRequest, ChangeRoleRequest, TeamCreateRequest, TeamRole, TeamUpdateRequest,
};

/// Parse a CLI role string into the `TeamRole` enum (snake_case wire form).
fn parse_role(s: &str) -> Result<TeamRole> {
    match s {
        "owner" => Ok(TeamRole::Owner),
        "maintainer" => Ok(TeamRole::Maintainer),
        "member" => Ok(TeamRole::Member),
        "watcher" => Ok(TeamRole::Watcher),
        other => Err(TemperError::Api(format!(
            "invalid role '{other}' (expected owner/maintainer/member/watcher)"
        ))),
    }
}

/// Invite an email to a team (owner/maintainer).
pub async fn invite_remote(
    client: &temper_client::TemperClient,
    team: &str,
    email: &str,
    role: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let req = temper_core::types::invitation::CreateInvitationRequest {
        invited_email: email.to_owned(),
        role: parse_role(role)?,
    };
    let inv = client
        .teams()
        .invite(team_id, &req)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&inv, fmt)?);
    Ok(())
}

/// Accept a team invitation by its token.
pub async fn accept_invitation(
    client: &temper_client::TemperClient,
    token: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let resp = client
        .teams()
        .accept_invitation(token)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&resp, fmt)?);
    Ok(())
}

/// Decline a team invitation by its token.
pub async fn decline_invitation(
    client: &temper_client::TemperClient,
    token: &str,
    _fmt: crate::format::OutputFormat,
) -> Result<()> {
    client
        .teams()
        .decline_invitation(token)
        .await
        .map_err(crate::commands::client_err)?;
    output::success("Invitation declined.");
    Ok(())
}

/// List pending invitations for a team (owner/maintainer).
pub async fn list_invitations_remote(
    client: &temper_client::TemperClient,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let invitations = client
        .teams()
        .list_invitations(team_id)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&invitations, fmt)?);
    Ok(())
}

/// Create a team on the remote server and render the resulting row.
pub async fn create_remote(
    client: &temper_client::TemperClient,
    slug: &str,
    name: Option<&str>,
    parent: Option<&str>,
    auto_join_role: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let auto_join_role = auto_join_role.map(parse_role).transpose()?;
    let req = TeamCreateRequest {
        slug: slug.to_owned(),
        name: name.map(str::to_owned),
        parent: parent.map(str::to_owned),
        auto_join_role,
    };

    let team = client
        .teams()
        .create(&req)
        .await
        .map_err(crate::commands::client_err)?;

    let rendered = crate::format::render(&team, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Add a member to a team (by team UUID + profile UUID) and render the row.
pub async fn add_member_remote(
    client: &temper_client::TemperClient,
    team: &str,
    profile: &str,
    role: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = uuid::Uuid::parse_str(team)
        .map_err(|e| TemperError::Api(format!("invalid team id '{team}': {e}")))?;
    let profile_id = uuid::Uuid::parse_str(profile)
        .map_err(|e| TemperError::Api(format!("invalid profile id '{profile}': {e}")))?;
    let req = AddMemberRequest {
        profile_id,
        role: parse_role(role)?,
    };

    let member = client
        .teams()
        .add_member(team_id, &req)
        .await
        .map_err(crate::commands::client_err)?;

    let rendered = crate::format::render(&member, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Update a team's metadata (name/description) and render the resulting row.
pub async fn update_remote(
    client: &temper_client::TemperClient,
    team: &str,
    name: Option<&str>,
    description: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    if name.is_none() && description.is_none() {
        return Err(TemperError::Api(
            "nothing to update: pass --name and/or --description".to_string(),
        ));
    }
    let team_id = resolve_team_id(client, team).await?;
    let req = TeamUpdateRequest {
        name: name.map(str::to_owned),
        description: description.map(str::to_owned),
    };
    let row = client
        .teams()
        .update(team_id, &req)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}

/// Soft-delete a team (owner only).
pub async fn delete_remote(
    client: &temper_client::TemperClient,
    team: &str,
    _fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    client
        .teams()
        .delete(team_id)
        .await
        .map_err(crate::commands::client_err)?;
    output::success("Team deleted.");
    Ok(())
}

/// Bulk-reassign a departing member's team-scoped resources (owner/maintainer).
/// Reassigns every resource owned by `from` and homed in a context shared to the
/// team, over to `to` (who must be a team member). `from`/`to` are profile UUIDs.
pub async fn reassign_remote(
    client: &temper_client::TemperClient,
    team: &str,
    from: &str,
    to: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let from_profile_id = uuid::Uuid::parse_str(from.trim())
        .map_err(|e| TemperError::Api(format!("invalid from id '{from}': {e}")))?;
    let to_profile_id = uuid::Uuid::parse_str(to.trim())
        .map_err(|e| TemperError::Api(format!("invalid to id '{to}': {e}")))?;
    let req = temper_core::types::reassign::BulkReassignRequest {
        from_profile_id,
        to_profile_id,
    };
    let ack = client
        .teams()
        .reassign(team_id, &req)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&ack, fmt)?);
    Ok(())
}

/// List the teams the caller is a member of and render them.
pub async fn list_remote(
    client: &temper_client::TemperClient,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let teams = client
        .teams()
        .list()
        .await
        .map_err(crate::commands::client_err)?;

    let rendered = crate::format::render(&teams, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Show a team's detail + members.
pub async fn show_remote(
    client: &temper_client::TemperClient,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let detail = client
        .teams()
        .get(team_id)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&detail, fmt)?);
    Ok(())
}

/// Leave a team you are a member of (self-removal).
pub async fn leave_remote(
    client: &temper_client::TemperClient,
    team: &str,
    _fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let me = client
        .profile()
        .get()
        .await
        .map_err(crate::commands::client_err)?;
    let outcome = client
        .teams()
        .remove_member(team_id, me.id)
        .await
        .map_err(crate::commands::client_err)?;
    output::success("You have left the team.");
    print_residual_nudge(team, &me.id.to_string(), &outcome.residual_owned);
    Ok(())
}

/// Remove a member from a team (owner/maintainer).
pub async fn remove_member_remote(
    client: &temper_client::TemperClient,
    team: &str,
    profile: &str,
    _fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let profile_id = uuid::Uuid::parse_str(profile)
        .map_err(|e| TemperError::Api(format!("invalid profile id '{profile}': {e}")))?;
    let outcome = client
        .teams()
        .remove_member(team_id, profile_id)
        .await
        .map_err(crate::commands::client_err)?;
    output::success("Member removed.");
    print_residual_nudge(team, profile, &outcome.residual_owned);
    Ok(())
}

/// On a non-empty residual reach, nudge toward the existing ownership handoff.
/// Owned resources in the team's contexts keep the removed member as their owner
/// (and, post-D1, their access) until handed off — surface, don't sweep.
fn print_residual_nudge(
    team: &str,
    from: &str,
    reach: &temper_core::types::reassign::ResidualOwnedReach,
) {
    if reach.count == 0 {
        return;
    }
    let ctxs: Vec<&str> = reach
        .contexts
        .iter()
        .map(|c| c.context_ref.as_str())
        .collect();
    output::warning(format!(
        "{} still owns {} resource(s) in: {}. Hand them off with:\n  \
         temper team reassign {} --from {} --to <member-uuid>",
        from,
        reach.count,
        ctxs.join(", "),
        team,
        from,
    ));
}

/// Change a member's role (owner/maintainer).
pub async fn set_role_remote(
    client: &temper_client::TemperClient,
    team: &str,
    profile: &str,
    role: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let profile_id = uuid::Uuid::parse_str(profile)
        .map_err(|e| TemperError::Api(format!("invalid profile id '{profile}': {e}")))?;
    let req = ChangeRoleRequest {
        role: parse_role(role)?,
    };
    let member = client
        .teams()
        .change_role(team_id, profile_id, &req)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&member, fmt)?);
    Ok(())
}
