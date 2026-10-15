//! Invitation tools — the team-invitation loop, both sides, through the network door.
//!
//! # Where this sits on the admin line
//!
//! The 64→28 consolidation removed invitations from MCP along with grants and the admin ledger.
//! That grouped two different kinds of "admin" together. Governing who may use the deployment
//! at all stays off this surface.
//! Team stewardship — an owner or maintainer running their own team — is ordinary work that an
//! MCP-only principal (a claude.ai connector user, with no CLI) has no other way to do. Without
//! these tools such a principal could not see an invitation addressed to them, let alone redeem
//! it.
//!
//! # Two tools, read and write separable
//!
//! - `invitation_read` — `mine` (your own pending invitations, tokens included), `count` (how
//!   many, with no token crossing the wire), `team` (a team's pending invitations, for its
//!   owners and maintainers).
//! - `invitation_manage` — `accept` / `decline` (yours, by token), `create` / `revoke` (a
//!   team's, as its owner or maintainer).
//!
//! # Authorization is the API's
//!
//! Every act forwards to its `/api/invitations` or `/api/teams/{id}/invit…` route on the
//! caller's own bearer; this module adds no gate of its own. `mine`, `count`, `accept` and
//! `decline` sit on the API's un-gated router, so a principal whose access to an invite-only
//! instance is not yet approved can still see and answer invitations, exactly as through the
//! CLI. Answering one does not approve that access: `has_system_access` reads the principal's
//! standing alone, and team membership (the gating team's included) does not set it. A machine
//! principal is refused every one of them at the API (it holds no verified email, so it is
//! never an invitee, and it does not steward teams); the refusal arrives as `ForbiddenDetail`
//! and is carried through in the server's own words.
//!
//! # Tokens
//!
//! The redemption token rides in the tool arguments and on to the request **body**, never a
//! path (see `InvitationTokenRequest`). `count` exists so that asking "is anything waiting?"
//! moves no token at all; reach for `mine` only to act on one.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_client::error::ClientError;
use temper_client::TemperClient;
use temper_core::types::invitation::CreateInvitationRequest;
use temper_core::types::team::TeamRole;

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn text_result(text: String) -> CallToolResult {
    CallToolResult::success(vec![rmcp::model::ContentBlock::text(text)])
}

fn required<T>(value: Option<T>, act: &str, field: &str) -> Result<T, rmcp::ErrorData> {
    value.ok_or_else(|| rmcp::ErrorData::invalid_params(format!("{act} requires `{field}`"), None))
}

/// The requirement clause for a team-side act's bare `403`. The wire's `403` carries no
/// sentence, so the tool names the gate: `invitation_service`'s `can_manage` on the team.
const TEAM_STEWARD_REQUIREMENT: &str = "you manage the team (owner or maintainer)";

/// Map an invitation act's error onto an MCP error.
///
/// The caller-actionable faces — the bare `403` (team-side acts: not an owner or maintainer),
/// `ForbiddenDetail` (a machine principal, or an invitation addressed to someone else — the
/// server's sentence names which), `404` (unknown token, or no such pending invitation), `409`
/// (a duplicate pending invite; an invitation already redeemed by someone else) and `400`
/// (expired, declined, revoked, an uninvitable role) — become invalid-params or invalid-request
/// carrying the server's own sentence. Everything else is the tool's internal fault voice.
fn map_invitation_err(act: &str, err: ClientError) -> rmcp::ErrorData {
    match err {
        ClientError::Forbidden => rmcp::ErrorData::invalid_params(
            format!("{act} requires that {TEAM_STEWARD_REQUIREMENT}"),
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

/// Resolve a team ref to its id, the way the CLI's `resolve_team_id` does: a team UUID or a
/// decorated `slug-<uuid>` ref carries the id itself; a bare slug (`+`-prefixed or not) is
/// matched against the teams the caller is a member of. A team the caller does not belong to
/// can only be named by its UUID.
async fn resolve_team(
    client: &TemperClient,
    act: &str,
    team: &str,
) -> Result<Uuid, rmcp::ErrorData> {
    let team_ref = team.strip_prefix('+').unwrap_or(team);
    // On a bare slug `parse_ref` errors with a resource-shaped message; swallow it and fall
    // through to the slug lookup, so the only refusal a caller sees is the team-shaped one below.
    if let Ok(id) = temper_workflow::operations::parse_ref(team_ref) {
        return Ok(id.uuid());
    }
    let teams = client
        .teams()
        .list()
        .await
        .across_auth(|e| map_invitation_err(act, e))?;
    teams
        .into_iter()
        .find(|t| t.slug == team_ref)
        .map(|t| t.id)
        .ok_or_else(|| {
            rmcp::ErrorData::invalid_params(
                format!(
                    "not a team: {team_ref:?} (expected a team slug, a decorated `slug-<uuid>` ref, \
                     or a team UUID; {team_ref:?} matched no team you are a member of)"
                ),
                None,
            )
        })
}

/// `count`'s team filter is a slug — the API counts invitations to a team the caller is
/// usually NOT yet a member of, so there is no listing to resolve a UUID against. A UUID would
/// silently match nothing, so it is refused rather than answered with a misleading `0`.
fn count_team_slug(team: Option<&str>) -> Result<Option<&str>, rmcp::ErrorData> {
    let Some(team) = team else {
        return Ok(None);
    };
    let slug = team.strip_prefix('+').unwrap_or(team);
    if Uuid::parse_str(slug).is_ok() {
        return Err(rmcp::ErrorData::invalid_params(
            "count's `team` takes a team slug, not a UUID (an invitation is usually to a team you \
             are not yet a member of, so a UUID cannot be resolved for you)"
                .to_string(),
            None,
        ));
    }
    Ok(Some(slug))
}

// ── Read ──────────────────────────────────────────────────────────────────────────────────

/// The invitation-read view to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum InvitationReadView {
    /// Your own pending invitations, across every team, each with the token that redeems it.
    Mine,
    /// How many pending invitations are addressed to you — integers only, no tokens.
    Count,
    /// A team's pending invitations (owner or maintainer of that team).
    Team,
}

/// Consolidated invitation-read tool — one read tool with a `view` discriminator.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct InvitationReadInput {
    /// Which invitation read to perform.
    pub view: InvitationReadView,
    /// For `team` (required): the team, as a slug (`+`-prefix optional), a decorated
    /// `slug-<uuid>` ref, or a UUID. For `count` (optional): a team SLUG, which additionally
    /// answers how many of your invitations are to that team (`matching`). Ignored for `mine`.
    #[serde(default)]
    pub team: Option<String>,
}

/// Dispatch the consolidated invitation-read tool.
pub async fn invitation_read(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: InvitationReadInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.view {
        InvitationReadView::Mine => {
            let rows = svc
                .relay_client(parts)?
                .teams()
                .list_my_invitations()
                .await
                .across_auth(|e| map_invitation_err("invitation_read mine", e))?;
            Ok(text_result(to_text(&rows)))
        }
        InvitationReadView::Count => {
            let team_slug = count_team_slug(input.team.as_deref())?;
            let counts = svc
                .relay_client(parts)?
                .teams()
                .count_my_invitations(team_slug)
                .await
                .across_auth(|e| map_invitation_err("invitation_read count", e))?;
            Ok(text_result(to_text(&counts)))
        }
        InvitationReadView::Team => {
            const ACT: &str = "invitation_read team";
            let team = required(input.team, ACT, "team")?;
            let client = svc.relay_client(parts)?;
            let team_id = resolve_team(&client, ACT, &team).await?;
            let rows = client
                .teams()
                .list_invitations(team_id)
                .await
                .across_auth(|e| map_invitation_err(ACT, e))?;
            Ok(text_result(to_text(&rows)))
        }
    }
}

// ── Write ─────────────────────────────────────────────────────────────────────────────────

/// The role an invitation confers, as the tool declares it.
///
/// A tool-local mirror of [`TeamRole`] because a tool's input schema must be self-contained (no
/// `$ref`/`$defs` on the wire), and `TeamRole`'s schema is not inlined: marking it so in
/// `temper-core` would also reshape the payload schemas the `scenario-schema` build stamps
/// wherever the two features unify. Which of these the caller may confer is the API's rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum InvitationRole {
    Owner,
    Maintainer,
    Member,
    Watcher,
}

impl From<InvitationRole> for TeamRole {
    fn from(role: InvitationRole) -> Self {
        match role {
            InvitationRole::Owner => TeamRole::Owner,
            InvitationRole::Maintainer => TeamRole::Maintainer,
            InvitationRole::Member => TeamRole::Member,
            InvitationRole::Watcher => TeamRole::Watcher,
        }
    }
}

/// The invitation-manage action to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum InvitationManageAction {
    /// Accept an invitation addressed to you, joining the team.
    Accept,
    /// Decline an invitation addressed to you.
    Decline,
    /// Invite an email address to a team (owner or maintainer).
    Create,
    /// Withdraw a team's pending invitation (owner or maintainer).
    Revoke,
}

/// Consolidated invitation-manage tool — one write tool with an `action` discriminator.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct InvitationManageInput {
    /// Which invitation action to perform.
    pub action: InvitationManageAction,
    /// The invitation's redemption token, from `invitation_read` view `mine`. Required for
    /// `accept` and `decline`. Only the account whose verified email the invitation names can
    /// use it.
    #[serde(default)]
    pub token: Option<String>,
    /// The team, as a slug (`+`-prefix optional), a decorated `slug-<uuid>` ref, or a UUID.
    /// Required for `create` and `revoke`.
    #[serde(default)]
    pub team: Option<String>,
    /// The email address to invite. Required for `create`. Nothing is mailed: the invitee finds
    /// the invitation themselves once they sign in with an account verified for this address.
    #[serde(default)]
    pub email: Option<String>,
    /// The role the invitee joins with. Required for `create`. Which roles you may confer is the
    /// API's rule; a refused role comes back with the server's reason.
    #[serde(default)]
    pub role: Option<InvitationRole>,
    /// The invitation's id, from `invitation_read` view `team`. Required for `revoke`.
    #[serde(default)]
    pub invitation_id: Option<Uuid>,
}

/// Dispatch the consolidated invitation-manage tool.
pub async fn invitation_manage(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: InvitationManageInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.action {
        InvitationManageAction::Accept => {
            const ACT: &str = "invitation_manage accept";
            let token = required(input.token, ACT, "token")?;
            let joined = svc
                .relay_client(parts)?
                .teams()
                .accept_invitation(&token)
                .await
                .across_auth(|e| map_invitation_err(ACT, e))?;
            Ok(text_result(to_text(&joined)))
        }
        InvitationManageAction::Decline => {
            const ACT: &str = "invitation_manage decline";
            let token = required(input.token, ACT, "token")?;
            svc.relay_client(parts)?
                .teams()
                .decline_invitation(&token)
                .await
                .across_auth(|e| map_invitation_err(ACT, e))?;
            Ok(text_result("Invitation declined.".to_string()))
        }
        InvitationManageAction::Create => {
            const ACT: &str = "invitation_manage create";
            let team = required(input.team, ACT, "team")?;
            let invited_email = required(input.email, ACT, "email")?;
            let role = required(input.role, ACT, "role")?;
            let client = svc.relay_client(parts)?;
            let team_id = resolve_team(&client, ACT, &team).await?;
            let invitation = client
                .teams()
                .invite(
                    team_id,
                    &CreateInvitationRequest {
                        invited_email,
                        role: role.into(),
                    },
                )
                .await
                .across_auth(|e| map_invitation_err(ACT, e))?;
            Ok(text_result(to_text(&invitation)))
        }
        InvitationManageAction::Revoke => {
            const ACT: &str = "invitation_manage revoke";
            let team = required(input.team, ACT, "team")?;
            let invitation_id = required(input.invitation_id, ACT, "invitation_id")?;
            let client = svc.relay_client(parts)?;
            let team_id = resolve_team(&client, ACT, &team).await?;
            client
                .teams()
                .revoke_invitation(team_id, invitation_id)
                .await
                .across_auth(|e| map_invitation_err(ACT, e))?;
            Ok(text_result("Invitation revoked.".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_input_deserializes_each_view() {
        let mine: InvitationReadInput =
            serde_json::from_value(serde_json::json!({ "view": "mine" })).unwrap();
        assert!(matches!(mine.view, InvitationReadView::Mine));
        assert!(mine.team.is_none());

        let count: InvitationReadInput =
            serde_json::from_value(serde_json::json!({ "view": "count", "team": "+platform" }))
                .unwrap();
        assert!(matches!(count.view, InvitationReadView::Count));
        assert_eq!(count.team.as_deref(), Some("+platform"));
    }

    #[test]
    fn manage_input_deserializes_create() {
        let input: InvitationManageInput = serde_json::from_value(serde_json::json!({
            "action": "create",
            "team": "platform",
            "email": "person@example.com",
            "role": "member",
        }))
        .unwrap();
        assert!(matches!(input.action, InvitationManageAction::Create));
        assert_eq!(input.role, Some(InvitationRole::Member));
        assert_eq!(input.email.as_deref(), Some("person@example.com"));
    }

    /// The mirror reads exactly the wire names `TeamRole` writes, so a role string an agent saw
    /// on any team row is one this tool accepts, and maps to the same role.
    #[test]
    fn every_invitation_role_is_the_team_role_of_the_same_name() {
        for role in [
            TeamRole::Owner,
            TeamRole::Maintainer,
            TeamRole::Member,
            TeamRole::Watcher,
        ] {
            let wire = serde_json::to_value(role).unwrap();
            let mirrored: InvitationRole = serde_json::from_value(wire).unwrap();
            assert_eq!(TeamRole::from(mirrored), role);
        }
    }

    #[test]
    fn count_takes_a_slug_and_refuses_a_uuid() {
        assert_eq!(count_team_slug(None).unwrap(), None);
        assert_eq!(
            count_team_slug(Some("+platform")).unwrap(),
            Some("platform")
        );
        assert_eq!(count_team_slug(Some("platform")).unwrap(), Some("platform"));
        let err = count_team_slug(Some(&Uuid::now_v7().to_string())).unwrap_err();
        assert!(err.message.contains("slug, not a UUID"), "{err}");
    }

    #[test]
    fn the_bare_403_names_the_steward_gate_and_a_detailed_403_keeps_its_sentence() {
        let bare = map_invitation_err("invitation_manage create", ClientError::Forbidden);
        assert_eq!(bare.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert_eq!(
            bare.message,
            "invitation_manage create requires that you manage the team (owner or maintainer)"
        );

        let detailed = map_invitation_err(
            "invitation_manage accept",
            ClientError::ForbiddenDetail {
                message: "this invitation is addressed to someone else".to_string(),
            },
        );
        assert_eq!(detailed.code, rmcp::model::ErrorCode::INVALID_REQUEST);
        assert!(detailed.message.ends_with("addressed to someone else"));
    }

    #[test]
    fn caller_actionable_faces_are_not_internal_faults() {
        for err in [
            ClientError::NotFound {
                message: "invitation not found, already used, or expired".to_string(),
            },
            ClientError::Conflict {
                message: "Conflict: a pending invitation already exists for this email".to_string(),
            },
            ClientError::Server {
                status: 400,
                message: "Bad request: invitation has expired".to_string(),
            },
        ] {
            let mapped = map_invitation_err("invitation_manage accept", err);
            assert_eq!(
                mapped.code,
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "{mapped}"
            );
            assert!(!mapped.message.starts_with("Conflict: "), "{mapped}");
            assert!(!mapped.message.starts_with("Bad request: "), "{mapped}");
        }
    }
}
