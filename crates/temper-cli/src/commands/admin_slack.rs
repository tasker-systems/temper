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

    let was_linked = row.was_linked;
    let idp_revoked = row.idp_revoked;
    println!("{}", crate::format::render(&row, fmt)?);

    if !was_linked {
        crate::output::warning(
            "No link existed for that principal — the disconnect was a no-op (this is not an error).",
        );
    }
    if !idp_revoked {
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
