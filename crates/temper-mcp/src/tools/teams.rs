//! Team tools — membership acts on teams, through the network door.
//!
//! `team_manage` carries `leave` today: the caller removes themselves from a team, as
//! `temper team leave <team>` does. It is the tool the rest of team stewardship joins (create,
//! update, add and remove members, change roles) once role-rank PR C settles the membership
//! writers those acts forward to.
//!
//! # Leaving needs no confirmation step
//!
//! Leaving over MCP is the same act as `temper team leave`: the same route on the caller's own
//! bearer. The human check on an agent's write is the client's approval of the call, which the
//! tool's annotations (`destructiveHint: true`) ask a client to apply. A leave the user did not
//! mean costs them their reach into the team until someone re-invites them; it gives no one else
//! anything. The API keeps the cases that cannot be undone that way: it refuses the last owner, a
//! membership the identity provider assigned, and your own personal team.
//!
//! # Authorization is the API's
//!
//! `leave` resolves the caller's own profile id (`GET /api/profile`) and forwards
//! `DELETE /api/teams/{id}/members/{profile_id}` with it. Self-removal is decided by the API's
//! membership rule; this module adds no gate. The route sits on the gated router, so a caller
//! whose access is not yet approved cannot leave on any surface; the system-access refusal
//! arrives in its own sentence.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_client::error::ClientError;
use temper_core::types::reassign::ResidualOwnedReach;

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

/// Map a team act's error onto an MCP error.
///
/// For a self-removal the bare `403` means the caller holds no membership the rule lets them act
/// from — in practice, they are not on the team. `ForbiddenDetail` (a machine principal) carries
/// the server's sentence. `404`, `409` (the last owner, an identity-provider membership, your own
/// personal team) and `400` carry the server's own sentence too. Everything else is the tool's
/// internal fault voice.
fn map_team_err(act: &str, err: ClientError) -> rmcp::ErrorData {
    match err {
        ClientError::Forbidden => rmcp::ErrorData::invalid_params(
            format!("{act} was refused: you can leave only a team you are a member of"),
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

/// The team-manage action to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum TeamManageAction {
    /// Leave a team you are a member of.
    Leave,
}

/// Consolidated team-manage tool — one write tool with an `action` discriminator.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TeamManageInput {
    /// Which team action to perform.
    pub action: TeamManageAction,
    /// The team, as a slug (`+`-prefix optional), a decorated `slug-<uuid>` ref, or a UUID.
    /// Required for `leave`.
    #[serde(default)]
    pub team: Option<String>,
}

/// Dispatch the consolidated team-manage tool.
pub async fn team_manage(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: TeamManageInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.action {
        TeamManageAction::Leave => {
            const ACT: &str = "team_manage leave";
            let team = required(input.team, ACT, "team")?;
            let client = svc.relay_client(parts)?;
            let team_id = resolve_team(&client, ACT, &team).await?;
            let me = client.profile().get().await.across_auth(|e| {
                rmcp::ErrorData::internal_error(
                    format!("{ACT}: could not read your own profile: {e}"),
                    None,
                )
            })?;
            let outcome = client
                .teams()
                .remove_member(team_id, me.id)
                .await
                .across_auth(|e| map_team_err(ACT, e))?;
            Ok(text_result(to_text(&LeaveAck {
                left: true,
                team_id,
                residual_owned: outcome.residual_owned,
            })))
        }
    }
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

    /// Only `leave` is offered: the stewardship acts join after role-rank PR C.
    #[test]
    fn leave_is_the_only_action() {
        for action in [
            "create",
            "add_member",
            "remove_member",
            "change_role",
            "delete",
        ] {
            let refused = serde_json::from_value::<TeamManageInput>(serde_json::json!({
                "action": action,
                "team": "acme-eng",
            }));
            assert!(refused.is_err(), "{action} must not deserialize");
        }
    }

    #[test]
    fn leave_refusals_are_the_callers_not_internal_faults() {
        let bare = map_team_err("team_manage leave", ClientError::Forbidden);
        assert_eq!(bare.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(bare.message.contains("only a team you are a member of"));

        let last_owner = map_team_err(
            "team_manage leave",
            ClientError::Conflict {
                message: "Conflict: a team keeps at least one owner".to_string(),
            },
        );
        assert_eq!(last_owner.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert_eq!(last_owner.message, "a team keeps at least one owner");

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
