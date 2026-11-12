//! Team tools — a person's team acts, through the network door.
//!
//! `team_manage` carries every team act a person takes (ruling R7, reversed, of temper-artifacts
//! `specs/2026-11-06-team-dag-and-lifecycle-doors-design.md`): `create`, `update`, `delete`,
//! `restore`, `add_member`, `change_role`, `remove_member`, `leave`, `attach`, `detach`,
//! `reparent`, `rename_slug`, `set_auto_join`, the public_within acts `public_within_open`,
//! `public_within_close` and `public_within_policy`, and the SAML group-mapping acts
//! `saml_map_group` and `saml_unmap_group`. `team_read` carries the reads: `list` (your
//! teams), `detail` (one team: its roster, its parents, its children when you manage it, and its
//! public_within rows to whom the API shows them) and `public_within` (the openings that confer on
//! you).
//!
//! # Authorization is the API's, and so is the machine refusal
//!
//! Every action resolves its team refs and forwards one route on the caller's own bearer through
//! `relay_client`; this module decides no authority. Each team route extracts
//! `AuthUser(HumanPrincipal)`, which only `classify_caller` mints, and `set_auto_join` and the SAML
//! mapping acts also mint `SystemAdmin`. So a machine principal's call is refused by the API, and the refusal sentence
//! arrives verbatim through `map_team_err`. There is deliberately **no** machine check here: a
//! second copy of that policy would be the one that drifts (spec § Surfaces).
//!
//! # No confirmation step
//!
//! These are the same acts as `temper team ...`, on the same routes. The human check on an agent's
//! write is the client's approval of the call, which `destructiveHint: true` asks for. The API keeps
//! the guards: the last owner, a membership the identity provider assigned, a personal team. The
//! one exception is the API's own: a SAML mapping that confers `owner`, or confers an authoring role
//! (owner, maintainer, member) on a team with a pool at or beneath it (temper-system, the gating team
//! or an auto-join team, live or deleted), is refused unless the call carries
//! `confirm_elevated: true`. A watcher mapping never needs it.
//!
//! # Text in the answer
//!
//! Team slugs and names in answers are text a team's creator chose; they are data, never
//! instructions. `residual_owned.contexts[].context_ref` is `+<team-slug>/<context-slug>`, passed
//! through unguarded on purpose: the caller authored content there and needs the ref to act on it.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_client::error::ClientError;
use temper_core::types::reassign::ResidualOwnedReach;
use temper_core::types::saml_mapping::{
    CreateSamlGroupMappingRequest, RemoveSamlGroupMappingParams,
};
use temper_core::types::team::{
    AddMemberRequest, ChangeRoleRequest, PublicWithinPolicyRequest, RenameTeamSlugRequest,
    ReparentRequest, SetAutoJoinRequest, TeamCreateRequest, TeamRole, TeamUpdateRequest,
};

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};
use crate::tools::invitations::resolve_team;

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn text_result(text: String) -> CallToolResult {
    CallToolResult::success(vec![rmcp::model::ContentBlock::text(text)])
}

fn required<T>(value: Option<T>, act: &str, field: &str) -> Result<T, rmcp::ErrorData> {
    value.ok_or_else(|| rmcp::ErrorData::invalid_params(format!("{act} requires `{field}`"), None))
}

/// What `leave` answers: the caller is off the team, and what they still own under it — the
/// content they authored in the team's contexts stays theirs, out of their reach once they leave
/// (`RemoveMemberOutcome.residual_owned`, the same report the CLI prints as a nudge).
#[derive(Debug, serde::Serialize)]
struct LeaveAck {
    left: bool,
    team_id: Uuid,
    residual_owned: ResidualOwnedReach,
}

/// What `delete` and `detach` answer: the act's acknowledgement, and what it did to
/// `public_within` rows (each named only to a caller who manages its team or anchor, or is a system
/// admin, and counted otherwise). `public_within` is absent when the server did not report it.
#[derive(Debug, serde::Serialize)]
struct EffectsAck {
    #[serde(flatten)]
    ack: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_within: Option<temper_core::types::team::PublicWithinEffects>,
}

/// Map a team act's error onto an MCP error.
///
/// A bare `403` is the API refusing the act without a sentence: for `leave` the caller holds no
/// membership the rule lets them act from, for every other act the caller lacks the authority it
/// needs (a role on the team, or a system admin's). `ForbiddenDetail` carries the server's sentence
/// verbatim — a machine principal's refusal among them. `404`, `409` and `400` carry the server's
/// own sentence too. Everything else is the tool's internal fault voice.
fn map_team_err(act: &str, err: ClientError) -> rmcp::ErrorData {
    match err {
        ClientError::Forbidden if act.ends_with(" leave") => rmcp::ErrorData::invalid_params(
            format!("{act} was refused: you can leave only a team you are a member of"),
            None,
        ),
        ClientError::Forbidden => rmcp::ErrorData::invalid_params(
            format!(
                "{act} was refused: you do not hold the role this act needs on the team(s) it \
                 names (or it is a system admin's act)"
            ),
            None,
        ),
        ClientError::ForbiddenDetail { message } => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{act}: {message}"),
            None,
        ),
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::Conflict { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        other => rmcp::ErrorData::internal_error(format!("{act} failed: {other}"), None),
    }
}

/// A team role, as `add_member` and `change_role` take it: owner, maintainer, member or watcher.
/// A mirror of [`TeamRole`] inlined into the tool's schema, which carries no `$ref` (the wire rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    Owner,
    Maintainer,
    Member,
    Watcher,
}

impl From<MemberRole> for TeamRole {
    fn from(role: MemberRole) -> Self {
        match role {
            MemberRole::Owner => TeamRole::Owner,
            MemberRole::Maintainer => TeamRole::Maintainer,
            MemberRole::Member => TeamRole::Member,
            MemberRole::Watcher => TeamRole::Watcher,
        }
    }
}

/// An auto-join role: member or watcher (a role that governs a team is granted by a person).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum PoolRole {
    Member,
    Watcher,
}

impl From<PoolRole> for TeamRole {
    fn from(role: PoolRole) -> Self {
        match role {
            PoolRole::Member => TeamRole::Member,
            PoolRole::Watcher => TeamRole::Watcher,
        }
    }
}

/// The team-manage action to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum TeamManageAction {
    /// Create a team; you become its owner.
    Create,
    /// Change a team's display name or description.
    Update,
    /// Soft-delete a team (its owner, or a system admin).
    Delete,
    /// Restore a soft-deleted team (its owner, or a system admin).
    Restore,
    /// Add a profile to a team at a role.
    AddMember,
    /// Change a member's role.
    ChangeRole,
    /// Remove a member from a team.
    RemoveMember,
    /// Leave a team you are a member of.
    Leave,
    /// Attach a child team under a parent team.
    Attach,
    /// Detach a child team from a parent team.
    Detach,
    /// Move a team from one parent to another.
    Reparent,
    /// Change a team's slug.
    RenameSlug,
    /// Set, change or clear a team's auto-join role (a system admin's act).
    SetAutoJoin,
    /// Open a team's owned contexts to the people under one of its ancestors (`anchor`).
    PublicWithinOpen,
    /// Close an opening (a manager of either side, or a system admin).
    PublicWithinClose,
    /// Forbid or allow openings over a team and everything beneath it.
    PublicWithinPolicy,
    /// Map an IdP group to a role on `team` (a system admin's act). Needs `idp_key`, `group` and
    /// `role`; an `owner` role, or an authoring role onto a team with a pool at or beneath it
    /// (temper-system, the gating team or an auto-join team), also needs `confirm_elevated: true`.
    /// A watcher mapping never does.
    SamlMapGroup,
    /// Remove a SAML group mapping from `team` (a system admin's act). Needs `idp_key` and `group`.
    /// Memberships it already conferred are left to the IdP reconcile.
    SamlUnmapGroup,
}

impl TeamManageAction {
    fn act(self) -> &'static str {
        match self {
            Self::Create => "team_manage create",
            Self::Update => "team_manage update",
            Self::Delete => "team_manage delete",
            Self::Restore => "team_manage restore",
            Self::AddMember => "team_manage add_member",
            Self::ChangeRole => "team_manage change_role",
            Self::RemoveMember => "team_manage remove_member",
            Self::Leave => "team_manage leave",
            Self::Attach => "team_manage attach",
            Self::Detach => "team_manage detach",
            Self::Reparent => "team_manage reparent",
            Self::RenameSlug => "team_manage rename_slug",
            Self::SetAutoJoin => "team_manage set_auto_join",
            Self::PublicWithinOpen => "team_manage public_within_open",
            Self::PublicWithinClose => "team_manage public_within_close",
            Self::PublicWithinPolicy => "team_manage public_within_policy",
            Self::SamlMapGroup => "team_manage saml_map_group",
            Self::SamlUnmapGroup => "team_manage saml_unmap_group",
        }
    }
}

/// Consolidated team-manage tool — one write tool with an `action` discriminator.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TeamManageInput {
    /// Which team action to perform.
    pub action: TeamManageAction,
    /// The team acted on, as a slug (`+`-prefix optional), a decorated `slug-<uuid>` ref, or a UUID.
    /// A slug resolves only among teams you are a member of, and a soft-deleted team only by UUID.
    /// Required for every action but `create`; for `attach`/`detach` it is the PARENT, for
    /// `reparent` the team being moved.
    #[serde(default)]
    pub team: Option<String>,
    /// The child team (same forms as `team`). Required for `attach` and `detach`.
    #[serde(default)]
    pub child: Option<String>,
    /// `reparent`: the current parent to leave (a team ref; resolved by the API).
    #[serde(default)]
    pub from: Option<String>,
    /// `reparent`: the new parent to join (a team ref; resolved by the API).
    #[serde(default)]
    pub to: Option<String>,
    /// `create`: the new team's slug (required). `rename_slug`: the new slug (required).
    #[serde(default)]
    pub slug: Option<String>,
    /// `create` / `update`: the display name.
    #[serde(default)]
    pub name: Option<String>,
    /// `update`: the description.
    #[serde(default)]
    pub description: Option<String>,
    /// `create`: parent team refs (at most 16), resolved by the API.
    #[serde(default)]
    pub parents: Option<Vec<String>>,
    /// `add_member` / `change_role` / `remove_member`: the member's profile UUID (from
    /// team_read view `detail`).
    #[serde(default)]
    pub profile_id: Option<Uuid>,
    /// `add_member` / `change_role` / `saml_map_group`: owner, maintainer, member or watcher.
    #[serde(default)]
    pub role: Option<MemberRole>,
    /// `create` / `set_auto_join`: member or watcher; for `set_auto_join`, omit or null to clear.
    #[serde(default)]
    pub auto_join_role: Option<PoolRole>,
    /// `public_within_open` / `public_within_close`: the ancestor whose subtree reads the team's
    /// owned contexts (same forms as `team`). Required for both.
    #[serde(default)]
    pub anchor: Option<String>,
    /// `public_within_policy`: `true` forbids openings over the team and everything beneath it;
    /// `false` clears this team's forbid (a forbid set higher up still holds). Required.
    #[serde(default)]
    pub forbidden: Option<bool>,
    /// `saml_map_group` / `saml_unmap_group`: the IdP's key (`kb_saml_idp.idp_key`). Required.
    #[serde(default)]
    pub idp_key: Option<String>,
    /// `saml_map_group` / `saml_unmap_group`: the group value, exactly as the IdP asserts it.
    /// Required.
    #[serde(default)]
    pub group: Option<String>,
    /// `saml_map_group`: the admin's explicit confirmation, required for an `owner` role and for
    /// temper-system or the gating team as the target. Default `false`.
    #[serde(default)]
    pub confirm_elevated: bool,
}

/// Dispatch the consolidated team-manage tool.
pub async fn team_manage(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: TeamManageInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let act = input.action.act();
    let client = svc.relay_client(parts)?;
    let teams = client.teams();
    let err = |e| map_team_err(act, e);
    // The acted-on team, resolved the way every team tool resolves one.
    let team_id = match input.action {
        TeamManageAction::Create
        | TeamManageAction::PublicWithinOpen
        | TeamManageAction::PublicWithinClose => None,
        _ => {
            let team = required(input.team.clone(), act, "team")?;
            Some(resolve_team(&client, act, &team).await?)
        }
    };
    let team_id = || team_id.ok_or_else(|| rmcp::ErrorData::internal_error("no team", None));
    let text = match input.action {
        TeamManageAction::Create => to_text(
            &teams
                .create(&TeamCreateRequest {
                    slug: required(input.slug, act, "slug")?,
                    name: input.name,
                    parent: None,
                    parents: input.parents,
                    auto_join_role: input.auto_join_role.map(TeamRole::from),
                })
                .await
                .across_auth(err)?,
        ),
        TeamManageAction::Update => to_text(
            &teams
                .update(
                    team_id()?,
                    &TeamUpdateRequest {
                        name: input.name,
                        description: input.description,
                    },
                )
                .await
                .across_auth(err)?,
        ),
        TeamManageAction::Delete => {
            let effects = teams.delete(team_id()?).await.across_auth(err)?;
            to_text(&EffectsAck {
                ack: serde_json::json!({ "deleted": true, "team_id": team_id()? }),
                public_within: effects,
            })
        }
        TeamManageAction::Restore => to_text(&teams.restore(team_id()?).await.across_auth(err)?),
        TeamManageAction::AddMember => to_text(
            &teams
                .add_member(
                    team_id()?,
                    &AddMemberRequest {
                        profile_id: required(input.profile_id, act, "profile_id")?,
                        role: required(input.role, act, "role")?.into(),
                    },
                )
                .await
                .across_auth(err)?,
        ),
        TeamManageAction::ChangeRole => to_text(
            &teams
                .change_role(
                    team_id()?,
                    required(input.profile_id, act, "profile_id")?,
                    &ChangeRoleRequest {
                        role: required(input.role, act, "role")?.into(),
                    },
                )
                .await
                .across_auth(err)?,
        ),
        TeamManageAction::RemoveMember => to_text(
            &teams
                .remove_member(team_id()?, required(input.profile_id, act, "profile_id")?)
                .await
                .across_auth(err)?,
        ),
        TeamManageAction::Leave => {
            let me = client.profile().get().await.across_auth(|e| {
                rmcp::ErrorData::internal_error(
                    format!("{act}: could not read your own profile: {e}"),
                    None,
                )
            })?;
            let outcome = teams
                .remove_member(team_id()?, me.id)
                .await
                .across_auth(err)?;
            to_text(&LeaveAck {
                left: true,
                team_id: team_id()?,
                residual_owned: outcome.residual_owned,
            })
        }
        TeamManageAction::Attach | TeamManageAction::Detach => {
            let child = required(input.child, act, "child")?;
            let child_id = if input.action == TeamManageAction::Detach {
                resolve_child(&client, act, team_id()?, &child).await?
            } else {
                resolve_team(&client, act, &child).await?
            };
            if input.action == TeamManageAction::Attach {
                to_text(
                    &teams
                        .attach_child(team_id()?, child_id)
                        .await
                        .across_auth(err)?,
                )
            } else {
                let effects = teams
                    .detach_child(team_id()?, child_id)
                    .await
                    .across_auth(err)?;
                to_text(&EffectsAck {
                    ack: serde_json::json!({
                        "detached": true, "parent_team_id": team_id()?, "child_team_id": child_id,
                    }),
                    public_within: effects,
                })
            }
        }
        TeamManageAction::Reparent => to_text(
            &teams
                .reparent(
                    team_id()?,
                    &ReparentRequest {
                        from: required(input.from, act, "from")?,
                        to: required(input.to, act, "to")?,
                    },
                )
                .await
                .across_auth(err)?,
        ),
        TeamManageAction::RenameSlug => to_text(
            &teams
                .rename_slug(
                    team_id()?,
                    &RenameTeamSlugRequest {
                        slug: required(input.slug, act, "slug")?,
                    },
                )
                .await
                .across_auth(err)?,
        ),
        TeamManageAction::SetAutoJoin => to_text(
            &teams
                .set_auto_join(
                    team_id()?,
                    &SetAutoJoinRequest {
                        role: input.auto_join_role.map(TeamRole::from),
                    },
                )
                .await
                .across_auth(err)?,
        ),
        TeamManageAction::PublicWithinOpen | TeamManageAction::PublicWithinClose => {
            let team = required(input.team, act, "team")?;
            let anchor = required(input.anchor, act, "anchor")?;
            let (team_id, anchor_id) =
                resolve_public_within_pair(&client, act, &team, &anchor).await?;
            if input.action == TeamManageAction::PublicWithinOpen {
                to_text(
                    &teams
                        .open_public_within(team_id, anchor_id)
                        .await
                        .across_auth(err)?,
                )
            } else {
                teams
                    .close_public_within(team_id, anchor_id)
                    .await
                    .across_auth(err)?;
                to_text(&serde_json::json!({
                    "closed": true, "team_id": team_id, "anchor_id": anchor_id,
                }))
            }
        }
        TeamManageAction::PublicWithinPolicy => to_text(
            &teams
                .set_public_within_policy(
                    team_id()?,
                    &PublicWithinPolicyRequest {
                        forbidden: required(input.forbidden, act, "forbidden")?,
                    },
                )
                .await
                .across_auth(err)?,
        ),
        TeamManageAction::SamlMapGroup => to_text(
            &client
                .admin()
                .create_saml_group_mapping(&CreateSamlGroupMappingRequest {
                    idp_key: required(input.idp_key, act, "idp_key")?,
                    group: required(input.group, act, "group")?,
                    team_id: team_id()?,
                    role: required(input.role, act, "role")?.into(),
                    confirm_elevated: input.confirm_elevated,
                })
                .await
                .across_auth(err)?,
        ),
        TeamManageAction::SamlUnmapGroup => to_text(
            &client
                .admin()
                .remove_saml_group_mapping(&RemoveSamlGroupMappingParams {
                    idp_key: required(input.idp_key, act, "idp_key")?,
                    group: required(input.group, act, "group")?,
                    team_id: team_id()?,
                })
                .await
                .across_auth(err)?,
        ),
    };
    Ok(text_result(text))
}

/// Resolve a `public_within` row's two teams through
/// [`temper_client::teams::TeamsClient::resolve_public_within_pair`], as `temper team
/// public-within` does. A failed listing or detail read is the caller's own refusal, never "not a
/// team".
async fn resolve_public_within_pair(
    client: &temper_client::TemperClient,
    act: &str,
    team: &str,
    anchor: &str,
) -> Result<(Uuid, Uuid), rmcp::ErrorData> {
    use temper_client::teams::PublicWithinPairError;
    match client
        .teams()
        .resolve_public_within_pair(team, anchor)
        .await
    {
        Ok(pair) => Ok(pair),
        Err(PublicWithinPairError::Client(e)) => Err(e).across_auth(|e| map_team_err(act, e)),
        Err(other) => Err(rmcp::ErrorData::invalid_params(
            format!("{act}: {other}"),
            None,
        )),
    }
}

/// Resolve `detach`'s child ref. A parent's manager need not be a member of the child, so a bare
/// slug is looked up among the parent's own children (its detail lists them for its managers)
/// before the caller's memberships, as `temper team detach` does.
async fn resolve_child(
    client: &temper_client::TemperClient,
    act: &str,
    parent_id: Uuid,
    child: &str,
) -> Result<Uuid, rmcp::ErrorData> {
    let token = child.strip_prefix('+').unwrap_or(child);
    if temper_workflow::operations::parse_ref(token).is_err() {
        if let Ok(detail) = client.teams().get(parent_id).await {
            if let Some(c) = detail.children.iter().find(|c| c.slug == token) {
                return Ok(c.id);
            }
        }
    }
    resolve_team(client, act, child).await
}

/// The team-read view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum TeamReadView {
    /// The teams you are a member of.
    List,
    /// One team: roster, parents, and children when you manage it.
    Detail,
    /// The public_within openings that confer on you right now.
    PublicWithin,
}

/// Consolidated team-read tool — one read tool with a `view` discriminator.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TeamReadInput {
    /// Which view to read.
    pub view: TeamReadView,
    /// `detail`: the team, as a slug (`+`-prefix optional), a decorated `slug-<uuid>` ref, or a
    /// UUID.
    #[serde(default)]
    pub team: Option<String>,
}

/// Dispatch the consolidated team-read tool.
pub async fn team_read(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: TeamReadInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;
    let text = match input.view {
        TeamReadView::List => {
            const ACT: &str = "team_read list";
            to_text(
                &client
                    .teams()
                    .list()
                    .await
                    .across_auth(|e| map_team_err(ACT, e))?,
            )
        }
        TeamReadView::Detail => {
            const ACT: &str = "team_read detail";
            let team = required(input.team, ACT, "team")?;
            let team_id = resolve_team(&client, ACT, &team).await?;
            to_text(
                &client
                    .teams()
                    .get(team_id)
                    .await
                    .across_auth(|e| map_team_err(ACT, e))?,
            )
        }
        TeamReadView::PublicWithin => {
            const ACT: &str = "team_read public_within";
            to_text(
                &client
                    .teams()
                    .list_public_within()
                    .await
                    .across_auth(|e| map_team_err(ACT, e))?,
            )
        }
    };
    Ok(text_result(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manage_input_deserializes_leave() {
        let input = serde_json::from_value::<TeamManageInput>(serde_json::json!({
            "action": "leave",
            "team": "+acme-eng",
        }))
        .expect("leave deserializes");
        assert!(matches!(input.action, TeamManageAction::Leave));
        assert_eq!(input.team.as_deref(), Some("+acme-eng"));
    }

    /// Every action deserializes with the fields it needs.
    #[test]
    fn every_action_deserializes() {
        use serde_json::json;
        let pid = Uuid::now_v7();
        for (value, action) in [
            (
                json!({"action": "create", "slug": "acme", "parents": ["+org"], "auto_join_role": "watcher"}),
                TeamManageAction::Create,
            ),
            (
                json!({"action": "update", "team": "acme", "name": "Acme"}),
                TeamManageAction::Update,
            ),
            (
                json!({"action": "delete", "team": "acme"}),
                TeamManageAction::Delete,
            ),
            (
                json!({"action": "restore", "team": pid.to_string()}),
                TeamManageAction::Restore,
            ),
            (
                json!({"action": "add_member", "team": "acme", "profile_id": pid, "role": "member"}),
                TeamManageAction::AddMember,
            ),
            (
                json!({"action": "change_role", "team": "acme", "profile_id": pid, "role": "watcher"}),
                TeamManageAction::ChangeRole,
            ),
            (
                json!({"action": "remove_member", "team": "acme", "profile_id": pid}),
                TeamManageAction::RemoveMember,
            ),
            (
                json!({"action": "leave", "team": "acme"}),
                TeamManageAction::Leave,
            ),
            (
                json!({"action": "attach", "team": "org", "child": "acme"}),
                TeamManageAction::Attach,
            ),
            (
                json!({"action": "detach", "team": "org", "child": "acme"}),
                TeamManageAction::Detach,
            ),
            (
                json!({"action": "reparent", "team": "acme", "from": "+org", "to": "+eng"}),
                TeamManageAction::Reparent,
            ),
            (
                json!({"action": "rename_slug", "team": "acme", "slug": "acme-2"}),
                TeamManageAction::RenameSlug,
            ),
            (
                json!({"action": "set_auto_join", "team": "acme", "auto_join_role": null}),
                TeamManageAction::SetAutoJoin,
            ),
            (
                json!({"action": "public_within_open", "team": "acme", "anchor": "+org"}),
                TeamManageAction::PublicWithinOpen,
            ),
            (
                json!({"action": "public_within_close", "team": "acme", "anchor": "+org"}),
                TeamManageAction::PublicWithinClose,
            ),
            (
                json!({"action": "public_within_policy", "team": "acme", "forbidden": true}),
                TeamManageAction::PublicWithinPolicy,
            ),
        ] {
            let input = serde_json::from_value::<TeamManageInput>(value.clone())
                .unwrap_or_else(|e| panic!("{value}: {e}"));
            assert_eq!(input.action, action, "{value}");
        }
        let input = serde_json::from_value::<TeamManageInput>(json!({
            "action": "reparent", "team": "acme", "from": "+org", "to": "+eng",
        }))
        .unwrap();
        assert_eq!(
            (input.from.as_deref(), input.to.as_deref()),
            (Some("+org"), Some("+eng"))
        );
        let read =
            serde_json::from_value::<TeamReadInput>(json!({"view": "detail", "team": "+acme"}))
                .unwrap();
        assert_eq!(read.view, TeamReadView::Detail);
        assert!(serde_json::from_value::<TeamManageInput>(json!({"action": "transfer"})).is_err());
    }

    /// A bare 403 on a non-leave act names the authority it lacks, not leaving.
    #[test]
    fn a_bare_refusal_names_the_act() {
        let refused = map_team_err("team_manage attach", ClientError::Forbidden);
        assert_eq!(refused.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(refused
            .message
            .starts_with("team_manage attach was refused"));
        assert!(!refused.message.contains("leave"), "{refused:?}");
    }

    #[test]
    fn leave_refusals_are_the_callers_not_internal_faults() {
        let bare = map_team_err("team_manage leave", ClientError::Forbidden);
        assert_eq!(bare.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(bare.message.contains("only a team you are a member of"));

        let last_owner = map_team_err(
            "team_manage leave",
            ClientError::Conflict {
                message:
                    "Conflict: cannot remove or demote the team's last owner; grant `owner` to \
                          another member first"
                        .to_string(),
            },
        );
        assert_eq!(last_owner.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            last_owner
                .message
                .starts_with("cannot remove or demote the team's last owner"),
            "{last_owner:?}"
        );

        let machine = map_team_err(
            "team_manage leave",
            ClientError::ForbiddenDetail {
                message: "this action is not available to a machine principal".to_string(),
            },
        );
        assert_eq!(machine.code, rmcp::model::ErrorCode::INVALID_REQUEST);
        assert!(machine.message.ends_with("machine principal"));
    }
}
