//! Team membership commands: join, status, leave; plus the lifecycle surface
//! (create, add-member, list) that round-trips CLI → client → API → service.

use crate::actions::cogmap::resolve_team_id;
use crate::error::{Result, TemperError};
use crate::output;
use temper_core::types::access_gate::JoinRequestStatus;
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

/// Submit a join request for a team (defaults to system gating team).
pub fn join(message: Option<&str>) -> crate::error::Result<()> {
    let message = message.map(|s| s.to_string());
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            match client
                .access()
                .create_request(message.as_deref(), "cli", None)
                .await
            {
                Ok(result) => {
                    output::success("Access request submitted.");
                    output::plain("  You'll gain access once an admin approves your request.");
                    output::hint("  Run `temper team status` to check.");
                    output::blank();
                    output::dim(format!("  Request ID: {}", result.id));
                }
                Err(temper_client::error::ClientError::Conflict { .. }) => {
                    output::warning("You already have a pending request.");
                    output::hint("  Run `temper team status` to check its status.");
                }
                Err(e) => return Err(crate::commands::client_err(e)),
            }

            Ok(())
        })
    })
}

/// Check the status of the caller's join request.
pub fn status() -> crate::error::Result<()> {
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let request = client
                .access()
                .get_own_request()
                .await
                .map_err(crate::commands::client_err)?;

            match request {
                None => {
                    output::plain("You haven't requested access yet.");
                    output::hint("Run `temper team join` to get started.");
                }
                Some(req) => match req.status {
                    JoinRequestStatus::Pending => {
                        output::plain(format!(
                            "Your request is pending review (submitted {}).",
                            req.created.format("%Y-%m-%d")
                        ));
                    }
                    JoinRequestStatus::Approved => {
                        let reviewed = req
                            .reviewed_at
                            .map(|d| d.format("%Y-%m-%d").to_string())
                            .unwrap_or_else(|| "unknown date".to_string());
                        output::success(format!("You have access. Approved on {reviewed}."));
                    }
                    JoinRequestStatus::Rejected => {
                        output::warning("Your previous request was not approved.");
                        output::hint(
                            "You can submit a new one with `temper team join --message \"...\"`.",
                        );
                    }
                    JoinRequestStatus::Withdrawn => {
                        output::plain("You withdrew your request.");
                        output::hint("Submit a new one with `temper team join --message \"...\"`.");
                    }
                },
            }

            Ok(())
        })
    })
}

/// Withdraw a pending join request.
pub fn withdraw_request() -> crate::error::Result<()> {
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let request = client
                .access()
                .get_own_request()
                .await
                .map_err(crate::commands::client_err)?;

            match request {
                None => {
                    output::plain("Nothing to leave — you don't have a pending request.");
                }
                Some(req) => match req.status {
                    JoinRequestStatus::Pending => {
                        client
                            .access()
                            .withdraw_request()
                            .await
                            .map_err(crate::commands::client_err)?;
                        output::success("Request withdrawn.");
                    }
                    JoinRequestStatus::Approved => {
                        output::plain("To leave a team after approval, contact an admin.");
                    }
                    _ => {
                        output::plain("Nothing to leave — no active request or membership.");
                    }
                },
            }

            Ok(())
        })
    })
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
    client
        .teams()
        .remove_member(team_id, me.id)
        .await
        .map_err(crate::commands::client_err)?;
    output::success("You have left the team.");
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
    client
        .teams()
        .remove_member(team_id, profile_id)
        .await
        .map_err(crate::commands::client_err)?;
    output::success("Member removed.");
    Ok(())
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
