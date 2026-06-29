//! Admin commands: system-settings show/update, promote, request review.
//! Round-trips CLI → AdminClient → API → access_service.

use crate::error::{Result, TemperError};
use temper_core::types::access_gate::JoinRequestStatus;
use temper_core::types::admin::{PromoteAdminRequest, UpdateSettingsRequest};

/// Show settings when no flag is set; otherwise PATCH and render the result.
///
/// The `UpdateSettingsRequest` is built in the `main.rs` dispatch arm and passed
/// by value — no per-field args needed here.
pub async fn settings_remote(
    client: &temper_client::TemperClient,
    req: UpdateSettingsRequest,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let settings = if req.is_empty() {
        client
            .admin()
            .get_settings()
            .await
            .map_err(crate::commands::client_err)?
    } else {
        client
            .admin()
            .update_settings(&req)
            .await
            .map_err(crate::commands::client_err)?
    };

    let rendered = crate::format::render(&settings, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Promote a profile to owner on a team (defaults to the gating team).
pub async fn promote_remote(
    client: &temper_client::TemperClient,
    profile: &str,
    team: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let profile_id = uuid::Uuid::parse_str(profile)
        .map_err(|e| TemperError::Api(format!("invalid profile id '{profile}': {e}")))?;

    // Resolve --team to a UUID when provided; None ⇒ server uses the gating team.
    let team_id = match team {
        Some(t) => Some(crate::actions::cogmap::resolve_team_id(client, t).await?),
        None => None,
    };

    let req = PromoteAdminRequest {
        profile_id,
        team_id,
    };
    let row = client
        .admin()
        .promote(&req)
        .await
        .map_err(crate::commands::client_err)?;

    let rendered = crate::format::render(&row, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// List pending join requests.
pub async fn requests_list_remote(
    client: &temper_client::TemperClient,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let rows = client
        .admin()
        .list_requests()
        .await
        .map_err(crate::commands::client_err)?;
    let rendered = crate::format::render(&rows, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Approve or reject a join request.
pub async fn requests_review_remote(
    client: &temper_client::TemperClient,
    id: &str,
    approve: bool,
    reject: bool,
    note: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let request_id = uuid::Uuid::parse_str(id)
        .map_err(|e| TemperError::Api(format!("invalid request id '{id}': {e}")))?;

    let decision = match (approve, reject) {
        (true, false) => JoinRequestStatus::Approved,
        (false, true) => JoinRequestStatus::Rejected,
        _ => {
            return Err(TemperError::Api(
                "specify exactly one of --approve or --reject".to_string(),
            ))
        }
    };

    let row = client
        .admin()
        .review_request(request_id, decision, note.map(str::to_owned))
        .await
        .map_err(crate::commands::client_err)?;

    let rendered = crate::format::render(&row, fmt)?;
    println!("{rendered}");
    Ok(())
}
