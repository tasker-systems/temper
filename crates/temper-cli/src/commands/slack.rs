use temper_core::types::slack::IdpRevocation;

use crate::error::Result;
use crate::format::OutputFormat;

/// Disconnect the caller's own Slack link(s).
///
/// Plural: one profile can hold a Slack principal per workspace, and the server unbinds all of
/// them in one call. The response names each one, so the user is told what actually happened
/// rather than a canned "disconnected".
///
/// Caveats go to stderr via `output::warning`, never to stdout — temper
/// defaults to JSON on a non-TTY stdout, and a hint on stdout corrupts it.
pub async fn disconnect_remote(
    client: &temper_client::TemperClient,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .slack()
        .disconnect_me()
        .await
        .map_err(crate::commands::client_err)?;

    // Render FIRST (JSON to stdout), then consume the payload for the stderr caveats.
    println!("{}", crate::format::render(&row, fmt)?);
    let disconnected = row.disconnected;

    if disconnected.is_empty() {
        crate::output::warning("No Slack link was found for your profile — nothing to disconnect.");
        return Ok(());
    }

    let names = disconnected
        .iter()
        .map(|d| d.slack_principal_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    crate::output::warning(&format!("Disconnected: {names}."));

    // Gated on `Failed` ONLY. `NotAttempted` means there was no stored grant to revoke — a
    // pre-T3 link — and warning about an unconfirmed revocation there tells the user their
    // (nonexistent) grant might still be live at the IdP, which is nonsense.
    if disconnected
        .iter()
        .any(|d| d.idp_revocation == IdpRevocation::Failed)
    {
        crate::output::warning(
            "The identity provider did not confirm revocation. Your stored grant was destroyed \
             regardless, so temper can no longer use it.",
        );
    }
    crate::output::warning(
        "Disconnect stops future access-token mints. An access token already issued remains \
         valid until it expires (up to one hour) — this is not an instant cutoff.",
    );
    Ok(())
}
