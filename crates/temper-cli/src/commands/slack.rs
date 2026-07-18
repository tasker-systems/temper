use crate::error::Result;
use crate::format::OutputFormat;

/// Disconnect the caller's own Slack link.
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

    let was_linked = row.was_linked;
    let idp_revoked = row.idp_revoked;
    println!("{}", crate::format::render(&row, fmt)?);

    if !was_linked {
        crate::output::warning("No Slack link was found for your profile — nothing to disconnect.");
        return Ok(());
    }
    if !idp_revoked {
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
