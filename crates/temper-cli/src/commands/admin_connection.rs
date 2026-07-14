//! `temper admin connection` — operator-only connection provisioning.
//!
//! Thin commands: parse, resolve refs to ids, call the client, render. `--owner-team` records
//! only the connection's OWNER, never its reach — owning a connection does not confer the right
//! to subscribe to it.

use temper_core::types::connection::{Connection, ProvisionConnectionRequest};

use crate::error::{Result, TemperError};
use crate::format::OutputFormat;

fn parse_uuid(what: &str, raw: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(raw).map_err(|e| TemperError::Api(format!("invalid {what} '{raw}': {e}")))
}

/// Say out loud what a freshly provisioned connection can and cannot do.
///
/// Invariant 6, at the one moment it is cheap to act on: a connection is born with no credential
/// and both capability tiers empty, and an operator who does not know that will wait for events
/// that can never arrive, or hand an agent reach it does not have. Absence of capability must be
/// loud.
fn announce_capabilities(row: &Connection) {
    if row.needs_credential() {
        crate::output::warning(
            "needs_credential — no credential is attached, so nothing can be received or reached yet.",
        );
    }
    if !row.is_ledger_capable() {
        crate::output::hint(
            "Not ledger-capable: no webhook events are registered, so no events will land.",
        );
    }
    if !row.is_reach_capable() {
        crate::output::hint(
            "Not reach-capable: the tool manifest is empty, so agents cannot read the remote back — \
             a subscription against this connection is legal and durable, but INERT FOR JUDGMENT.",
        );
    }
    if row.reach_granularity.is_none() || row.reach_covers.is_none() {
        crate::output::hint(
            "Reach fidelity is undeclared (--reach / --covers). A coarse connector is acceptable; \
             an UNDECLARED one is not — declare it before granting reach to a team.",
        );
    }
}

/// Provision a connection. Born `needs_credential`.
pub async fn provision_remote(
    client: &temper_client::TemperClient,
    provider: &str,
    name: &str,
    owner_team: Option<&str>,
    reach: Option<&str>,
    covers: Option<&str>,
    fmt: OutputFormat,
) -> Result<()> {
    let owner_team_id = match owner_team {
        Some(t) => Some(crate::actions::cogmap::resolve_team_id(client, t).await?),
        None => None,
    };

    let req = ProvisionConnectionRequest {
        provider: provider.to_string(),
        name: name.to_string(),
        owner_team_id,
        reach_granularity: reach.map(str::to_string),
        reach_covers: covers.map(str::to_string),
    };
    let row = client
        .connections()
        .provision(&req)
        .await
        .map_err(crate::commands::client_err)?;

    println!("{}", crate::format::render(&row, fmt)?);
    announce_capabilities(&row);
    Ok(())
}

/// Enumerate connections.
pub async fn list_remote(
    client: &temper_client::TemperClient,
    include_revoked: bool,
    fmt: OutputFormat,
) -> Result<()> {
    let rows = client
        .connections()
        .list(include_revoked)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&rows, fmt)?);
    Ok(())
}

/// Show one connection.
pub async fn show_remote(
    client: &temper_client::TemperClient,
    id: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .connections()
        .get(parse_uuid("connection id", id)?)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&row, fmt)?);
    announce_capabilities(&row);
    Ok(())
}

/// Revoke a connection.
pub async fn revoke_remote(
    client: &temper_client::TemperClient,
    id: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .connections()
        .revoke(parse_uuid("connection id", id)?)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&row, fmt)?);
    crate::output::hint(
        "The connection's profile, emitter entity, and home context were NOT removed — events \
         already attributed to this emitter must keep resolving.",
    );
    Ok(())
}
