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

/// Invite an email to a team
/// (a team manager or a system admin; on a personal team, its owner only).
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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    output::success("Invitation declined.");
    Ok(())
}

/// Revoke (withdraw) a pending invitation by its id
/// (a team manager or a system admin; on a personal team, its owner only).
/// The invitation id is shown by `temper team invitations`.
pub async fn revoke_invitation_remote(
    client: &temper_client::TemperClient,
    team: &str,
    invitation_id: &str,
    _fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let invitation_id = uuid::Uuid::parse_str(invitation_id.trim())
        .map_err(|e| TemperError::Api(format!("invalid invitation id '{invitation_id}': {e}")))?;
    client
        .teams()
        .revoke_invitation(team_id, invitation_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    output::success("Invitation revoked.");
    Ok(())
}

/// List pending invitations for a team
/// (a team manager or a system admin; on a personal team, its owner only).
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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&invitations, fmt)?);
    Ok(())
}

/// Create a team on the remote server and render the resulting row.
pub async fn create_remote(
    client: &temper_client::TemperClient,
    slug: &str,
    name: Option<&str>,
    parents: &[String],
    auto_join_role: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let auto_join_role = auto_join_role.map(parse_role).transpose()?;
    // One parent rides `parent`, which every server understands; several ride `parents`.
    let (parent, parents) = match parents {
        [] => (None, None),
        [one] => (Some(one.clone()), None),
        many => (None, Some(many.to_vec())),
    };
    let req = TeamCreateRequest {
        slug: slug.to_owned(),
        name: name.map(str::to_owned),
        parent,
        parents,
        auto_join_role,
    };

    let team = client
        .teams()
        .create(&req)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;

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
        .map_err(crate::actions::runtime::client_err_to_temper)?;

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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}

/// Soft-delete a team (its owner, or a system admin).
pub async fn delete_remote(
    client: &temper_client::TemperClient,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let effects = client
        .teams()
        .delete(team_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    output::success("Team deleted.");
    print_public_within_effects(effects.as_ref(), fmt)
}

/// Print what an act did to `public_within` rows, whenever the server reported it (empty lists
/// included, so the output's shape does not depend on what happened). `None` is a server that
/// predates the report: nothing is printed and nothing is claimed either way.
fn print_public_within_effects(
    effects: Option<&temper_core::types::team::PublicWithinEffects>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let Some(e) = effects else {
        return Ok(());
    };
    println!(
        "{}",
        crate::format::render(&serde_json::json!({ "public_within": e }), fmt)?
    );
    Ok(())
}

/// Detach a child team from a parent team (a manager of the parent or of the child, or a system
/// admin). The child keeps its roster and only this link is cut; its members stop reaching what is
/// attached to the parent.
///
/// The child is named by UUID, decorated ref, or slug. A slug is looked up among the parent's own
/// children (on the parent's detail, which lists them for its managers), because a parent's manager
/// need not be a member of the child.
pub async fn detach_remote(
    client: &temper_client::TemperClient,
    parent: &str,
    child: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let parent_id = resolve_team_id(client, parent).await?;
    let token = child.trim().strip_prefix('+').unwrap_or(child.trim());
    let child_id = match temper_workflow::operations::parse_ref(token) {
        Ok(id) => uuid::Uuid::from(id),
        Err(_) => {
            let detail = client
                .teams()
                .get(parent_id)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            detail
                .children
                .into_iter()
                .find(|c| c.slug == token)
                .map(|c| c.id)
                .ok_or_else(|| {
                    TemperError::Api(format!(
                        "not a child of {parent:?}: {token:?} (expected a child team slug, a \
                         decorated `slug-<uuid>` ref, or a team UUID)"
                    ))
                })?
        }
    };
    let effects = client
        .teams()
        .detach_child(parent_id, child_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    output::success("Child team detached.");
    print_public_within_effects(effects.as_ref(), fmt)
}

/// Attach a child team under a parent team (a manager of both; a temper-system or gating-team
/// parent, or a child with an auto-join or the gating team in or below it, a system admin).
pub async fn attach_remote(
    client: &temper_client::TemperClient,
    parent: &str,
    child: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let parent_id = resolve_team_id(client, parent).await?;
    let child_id = resolve_team_id(client, child).await?;
    let outcome = client
        .teams()
        .attach_child(parent_id, child_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&outcome, fmt)?);
    Ok(())
}

/// Move a team from one parent to another in one act. `from`/`to` are passed to the API as given
/// (it resolves them), so a parent the caller is not a member of is still nameable by slug.
pub async fn reparent_remote(
    client: &temper_client::TemperClient,
    team: &str,
    from: &str,
    to: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let outcome = client
        .teams()
        .reparent(
            team_id,
            &temper_core::types::team::ReparentRequest {
                from: from.to_owned(),
                to: to.to_owned(),
            },
        )
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&outcome, fmt)?);
    Ok(())
}

/// Restore a soft-deleted team (its owner, or a system admin), printing the revived parents,
/// children and roster.
pub async fn restore_remote(
    client: &temper_client::TemperClient,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let outcome = client
        .teams()
        .restore(team_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&outcome, fmt)?);
    Ok(())
}

/// Change a team's slug, warning that `+old/...` addresses stop resolving.
pub async fn rename_slug_remote(
    client: &temper_client::TemperClient,
    team: &str,
    slug: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let outcome = client
        .teams()
        .rename_slug(
            team_id,
            &temper_core::types::team::RenameTeamSlugRequest {
                slug: slug.to_owned(),
            },
        )
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&outcome, fmt)?);
    if outcome.from != outcome.to {
        output::warning(format!(
            "addresses changed: `+{}/...` refs no longer resolve; use `+{}/...` (UUID refs are \
             unaffected, and refs written in content are not rewritten)",
            outcome.from, outcome.to
        ));
    }
    Ok(())
}

/// Set, change or clear a team's auto-join role (a system admin). `role` is member, watcher or
/// none.
pub async fn set_auto_join_remote(
    client: &temper_client::TemperClient,
    team: &str,
    role: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let role = match role {
        "none" | "clear" => None,
        other => Some(parse_role(other)?),
    };
    let team_id = resolve_team_id(client, team).await?;
    let outcome = client
        .teams()
        .set_auto_join(
            team_id,
            &temper_core::types::team::SetAutoJoinRequest { role },
        )
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&outcome, fmt)?);
    Ok(())
}

/// Bulk-reassign a departing member's team-scoped resources (a manager of the team, or a system admin).
/// Reassigns every resource owned by `from` and homed in a context the team owns,
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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
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
        .map_err(crate::actions::runtime::client_err_to_temper)?;

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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    let outcome = client
        .teams()
        .remove_member(team_id, me.id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    output::success("You have left the team.");
    print_residual_nudge(team, &me.id.to_string(), &outcome.residual_owned);
    Ok(())
}

/// Remove a member from a team
/// (a team manager or a system admin; on a personal team, its owner only).
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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
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

/// Change a member's role (a team owner/maintainer or a system admin; personal team: its owner).
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
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&member, fmt)?);
    Ok(())
}

/// Resolve a `public_within` row's two teams through
/// [`temper_client::teams::TeamsClient::resolve_public_within_pair`]. A failed listing or detail
/// read is the caller's own error; only a ref that matches no readable team is "not a team".
pub async fn resolve_public_within_pair(
    client: &temper_client::TemperClient,
    team: &str,
    anchor: &str,
) -> Result<(uuid::Uuid, uuid::Uuid)> {
    use temper_client::teams::PublicWithinPairError;
    client
        .teams()
        .resolve_public_within_pair(team, anchor)
        .await
        .map_err(|e| match e {
            PublicWithinPairError::Client(e) => crate::actions::runtime::client_err_to_temper(e),
            other => TemperError::Api(other.to_string()),
        })
}

/// `temper team public-within ...`: open, close, forbid/allow, list.
pub async fn public_within(
    client: &temper_client::TemperClient,
    action: crate::cli::PublicWithinAction,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    use crate::cli::PublicWithinAction;
    let err = crate::actions::runtime::client_err_to_temper;
    match action {
        PublicWithinAction::Open { team, anchor } => {
            let (team_id, anchor_id) = resolve_public_within_pair(client, &team, &anchor).await?;
            let outcome = client
                .teams()
                .open_public_within(team_id, anchor_id)
                .await
                .map_err(err)?;
            println!("{}", crate::format::render(&outcome, fmt)?);
            output::warning(format!(
                "team {team_id}'s owned contexts are readable by {} people on {} teams under \
                 anchor {anchor_id} while this opening confers. That audience grows with the \
                 anchor's subtree: every manager of a team under it, identity-provider group sync \
                 and a system admin can add to it. Close it with `temper team public-within close \
                 {team_id} {anchor_id}`.",
                outcome.audience.principals, outcome.audience.teams
            ));
        }
        PublicWithinAction::Close { team, anchor } => {
            let (team_id, anchor_id) = resolve_public_within_pair(client, &team, &anchor).await?;
            client
                .teams()
                .close_public_within(team_id, anchor_id)
                .await
                .map_err(err)?;
            output::success("Opening closed.");
        }
        PublicWithinAction::Policy { team, setting } => {
            // clap's value_parser admits only `forbid` and `allow`.
            let forbidden = setting == "forbid";
            let team_id = resolve_team_id(client, &team).await?;
            let outcome = client
                .teams()
                .set_public_within_policy(
                    team_id,
                    &temper_core::types::team::PublicWithinPolicyRequest { forbidden },
                )
                .await
                .map_err(err)?;
            println!("{}", crate::format::render(&outcome, fmt)?);
        }
        PublicWithinAction::List => {
            let rows = client.teams().list_public_within().await.map_err(err)?;
            println!("{}", crate::format::render(&rows, fmt)?);
        }
    }
    Ok(())
}
