//! Invitee-side invitation commands: list the pending invitations addressed to
//! the authenticated caller (across all teams), resolved by email correlation.

use crate::error::Result;

/// List the caller's own pending team invitations.
pub async fn list_mine(
    client: &temper_client::TemperClient,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let invitations = client
        .teams()
        .list_my_invitations()
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&invitations, fmt)?);
    Ok(())
}
