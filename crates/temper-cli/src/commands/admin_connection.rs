//! `temper admin connection` — operator-only connection provisioning.
//!
//! Thin commands: parse, resolve refs to ids, call the client, render. `--owner-team` records
//! only the connection's OWNER, never its reach — owning a connection does not confer the right
//! to subscribe to it.

use temper_core::types::connection::{
    Connection, ConnectionCredential, CredentialVerification, ProvisionConnectionRequest,
};

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

/// Attach the credential. `needs_credential` flips off because the column became non-NULL — there
/// is no status to set.
///
/// `--broker` names the implementation behind the seam; it is never a bare connector id. The
/// connector id lives on the row, per instance, which is what lets a self-hosted operator use their
/// own connectors in their own Vercel team.
pub async fn attach_credential_remote(
    client: &temper_client::TemperClient,
    id: &str,
    broker: &str,
    connector: &str,
    installation: Option<&str>,
    fmt: OutputFormat,
) -> Result<()> {
    let credential = ConnectionCredential {
        broker: broker.to_string(),
        connector: connector.to_string(),
        installation: installation.map(str::to_string),
    };
    let row = client
        .connections()
        .attach_credential(parse_uuid("connection id", id)?, &credential)
        .await
        .map_err(crate::commands::client_err)?;

    println!("{}", crate::format::render(&row, fmt)?);
    announce_capabilities(&row.connection);
    announce_verification(&row.connection, &row.verification);
    Ok(())
}

/// Surface what minting once at attach time observed. The observed reach is placed
/// next to the DECLARED reach so a reviewer sees the gap — there is no computed
/// `exceeds` bool (the two scopes are incommensurable; B3 records the
/// acknowledgment). An unverified attach says why, out loud (invariant 6).
fn announce_verification(conn: &Connection, v: &CredentialVerification) {
    if v.verified {
        if let Some(reach) = &v.observed_reach {
            crate::output::hint(format!(
                "Verified by minting once. The credential's ACTUAL remote reach is {reach}. \
                 Declared reach is granularity={:?}, covers={:?}. Where the actual reach exceeds \
                 the declared, that gap is real and must be acknowledged before granting a team.",
                conn.reach_granularity, conn.reach_covers,
            ));
        }
    } else if let Some(note) = &v.note {
        crate::output::warning(format!("Credential recorded but NOT verified — {note}"));
    }
}

/// Register the remote event types. Non-empty ⇒ ledger-capable.
pub async fn set_webhook_events_remote(
    client: &temper_client::TemperClient,
    id: &str,
    events: Vec<String>,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .connections()
        .set_webhook_events(parse_uuid("connection id", id)?, events)
        .await
        .map_err(crate::commands::client_err)?;

    println!("{}", crate::format::render(&row, fmt)?);
    announce_capabilities(&row);
    Ok(())
}

/// Declare the read-only remote tools. Non-empty ⇒ reach-capable.
pub async fn set_tool_manifest_remote(
    client: &temper_client::TemperClient,
    id: &str,
    tools: Vec<String>,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .connections()
        .set_tool_manifest(parse_uuid("connection id", id)?, tools)
        .await
        .map_err(crate::commands::client_err)?;

    println!("{}", crate::format::render(&row, fmt)?);
    announce_capabilities(&row);
    Ok(())
}

/// Grant a TEAM read-reach on this connection. Owning a connection is NOT reaching it — this
/// writes an access grant so the team's members inherit read on what the connection receives.
/// Reach is read-only; it confers no write.
pub async fn grant_reach_remote(
    client: &temper_client::TemperClient,
    id: &str,
    team: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let team_id = crate::actions::cogmap::resolve_team_id(client, team).await?;
    let row = client
        .connections()
        .grant_reach(parse_uuid("connection id", id)?, team_id)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&row, fmt)?);
    crate::output::hint(
        "Read-reach granted: the team's members now inherit read on what this connection receives. \
         Reach is read-only — it confers no write.",
    );
    Ok(())
}

/// Revoke a team's read-reach on this connection. Idempotent — an absent grant is a no-op.
pub async fn revoke_reach_remote(
    client: &temper_client::TemperClient,
    id: &str,
    team: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let team_id = crate::actions::cogmap::resolve_team_id(client, team).await?;
    let row = client
        .connections()
        .revoke_reach(parse_uuid("connection id", id)?, team_id)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&row, fmt)?);
    crate::output::hint(
        "The team now has no read-reach on this connection (idempotent — an absent grant was a no-op).",
    );
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
    crate::output::warning(
        "Revocation stops temper from minting NEW tokens for this connection. It does NOT reach \
         the provider — any token already minted stays valid at the remote until it expires. If a \
         credential is believed compromised, rotate it at the provider too.",
    );
    Ok(())
}
