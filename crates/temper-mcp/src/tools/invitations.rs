//! Invitation tools — list your own pending invitations, and accept/decline by token.
//!
//! Services-direct reads/writes (via `svc.api_state.pool`), mirroring the other
//! tool modules. `list_my_invitations` is the invitee-side resolver; accept/decline
//! delegate to the existing token-bearer service methods.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::service::TemperMcpService;
use temper_core::types::ids::ProfileId;
use temper_services::services::invitation_service;

/// MCP input for accept_invitation.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AcceptInvitationInput {
    /// The invitation token to redeem.
    pub token: String,
}

/// MCP input for decline_invitation.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeclineInvitationInput {
    /// The invitation token to decline.
    pub token: String,
}

pub async fn list_my_invitations(
    svc: &TemperMcpService,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let invites =
        invitation_service::list_for_profile(&svc.api_state.pool, ProfileId::from(profile.id))
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to list invitations: {e}"), None)
            })?;
    let text = serde_json::to_string_pretty(&invites).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

pub async fn accept_invitation(
    svc: &TemperMcpService,
    input: AcceptInvitationInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let resp = invitation_service::accept_invitation(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        &input.token,
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to accept invitation: {e}"), None)
    })?;
    let text = serde_json::to_string_pretty(&resp).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

pub async fn decline_invitation(
    svc: &TemperMcpService,
    input: DeclineInvitationInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    invitation_service::decline_invitation(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        &input.token,
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to decline invitation: {e}"), None)
    })?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        "Invitation declined.".to_string(),
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_input_deserializes() {
        let input: AcceptInvitationInput =
            serde_json::from_value(serde_json::json!({ "token": "abc" })).unwrap();
        assert_eq!(input.token, "abc");
    }

    #[test]
    fn decline_input_deserializes() {
        let input: DeclineInvitationInput =
            serde_json::from_value(serde_json::json!({ "token": "xyz" })).unwrap();
        assert_eq!(input.token, "xyz");
    }
}
