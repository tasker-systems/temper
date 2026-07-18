use temper_core::types::slack::IdpRevocation;

use crate::error::Result;
use crate::format::OutputFormat;

/// Disconnect any Slack principal. Requires system admin.
///
/// The principal is opaque and has 2–4 segments — it is passed whole and never
/// split.
pub async fn disconnect_remote(
    client: &temper_client::TemperClient,
    principal: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .slack()
        .admin_disconnect(principal)
        .await
        .map_err(crate::commands::client_err)?;

    // Render FIRST (JSON to stdout), then consume the payload for the stderr caveats.
    println!("{}", crate::format::render(&row, fmt)?);
    let disconnected = row.disconnected;

    if disconnected.is_empty() {
        crate::output::warning(
            "No link existed for that principal — the disconnect was a no-op (this is not an error).",
        );
    } else {
        let names = disconnected
            .iter()
            .map(|d| d.slack_principal_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        crate::output::warning(&format!("Disconnected: {names}."));
    }

    // `Failed` ONLY — a principal with no stored grant reports `NotAttempted`, and warning
    // "revoke out-of-band" at an operator whose target never had a grant sends them hunting for
    // something that does not exist. This is the live bug the three-state enum replaced.
    if disconnected
        .iter()
        .any(|d| d.idp_revocation == IdpRevocation::Failed)
    {
        crate::output::warning(
            "The identity provider did not confirm revocation; revoke out-of-band if that matters. \
             The local grant was destroyed regardless.",
        );
    }
    crate::output::warning(
        "The profile, its teams and its resources are untouched — disconnect unbinds an identity, \
         it does not deactivate an account.",
    );
    Ok(())
}
