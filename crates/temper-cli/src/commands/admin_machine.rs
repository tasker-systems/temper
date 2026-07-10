//! `temper admin machine` — operator-only machine-principal registration.
//!
//! Thin commands: parse, resolve refs to ids, call the client, render. Reach
//! (`--team`, `--cogmap`) is explicit and repeatable and is never inferred from
//! `--owner-team`, which records only the machine's owner.

use temper_core::types::machine::{
    GrantSpec, ProvisionMachineRequest, RebindMachineRequest, TeamSpec,
};

use crate::error::{Result, TemperError};
use crate::format::OutputFormat;

/// Split `"<ref>"` or `"<ref>:<suffix>"` on the LAST colon. Neither a UUID nor a
/// decorated `slug-<uuid>` ref contains a colon, so this cannot mangle a ref.
fn split_spec(raw: &str) -> (String, Option<String>) {
    match raw.rsplit_once(':') {
        Some((head, tail)) => (head.to_string(), Some(tail.to_string())),
        None => (raw.to_string(), None),
    }
}

fn parse_uuid(what: &str, raw: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(raw).map_err(|e| TemperError::Api(format!("invalid {what} '{raw}': {e}")))
}

/// Register a machine principal.
pub async fn provision_remote(
    client: &temper_client::TemperClient,
    client_id: &str,
    label: &str,
    owner_team: Option<&str>,
    teams: &[String],
    cogmaps: &[String],
    fmt: OutputFormat,
) -> Result<()> {
    let owner_team_id = match owner_team {
        Some(t) => Some(crate::actions::cogmap::resolve_team_id(client, t).await?),
        None => None,
    };

    let mut team_specs = Vec::with_capacity(teams.len());
    for raw in teams {
        let (team_ref, role) = split_spec(raw);
        team_specs.push(TeamSpec {
            team_id: crate::actions::cogmap::resolve_team_id(client, &team_ref).await?,
            role: role.unwrap_or_else(|| "member".to_string()),
        });
    }

    let mut grant_specs = Vec::with_capacity(cogmaps.len());
    for raw in cogmaps {
        let (cogmap_ref, mode) = split_spec(raw);
        // `parse_ref` returns `Result<ResourceId, TemperError>`; `ResourceId` is a newtype
        // over `Uuid`. Resolution is trailing-UUID-only, so a stale slug half is harmless.
        let cogmap_id = temper_workflow::operations::parse_ref(&cogmap_ref)
            .map_err(|e| TemperError::Api(format!("invalid cogmap ref '{cogmap_ref}': {e}")))?
            .0;
        grant_specs.push(GrantSpec {
            cogmap_id,
            can_write: mode.as_deref() != Some("ro"),
        });
    }

    let req = ProvisionMachineRequest {
        client_id: client_id.to_string(),
        label: label.to_string(),
        owner_team_id,
        teams: team_specs,
        grants: grant_specs,
    };
    let row = client
        .machine_clients()
        .provision(&req)
        .await
        .map_err(crate::commands::client_err)?;

    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}

/// Rotate an application: bind a new client id to the existing agent profile.
pub async fn rebind_remote(
    client: &temper_client::TemperClient,
    from: &str,
    client_id: &str,
    label: &str,
    no_revoke_old: bool,
    fmt: OutputFormat,
) -> Result<()> {
    let from_id = parse_uuid("machine client id", from)?;
    let req = RebindMachineRequest {
        client_id: client_id.to_string(),
        from_machine_client_id: from_id,
        label: label.to_string(),
        keep_old_active: no_revoke_old,
    };
    let row = client
        .machine_clients()
        .rebind(from_id, &req)
        .await
        .map_err(crate::commands::client_err)?;

    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}

/// Enumerate registered clients.
pub async fn list_remote(
    client: &temper_client::TemperClient,
    include_revoked: bool,
    fmt: OutputFormat,
) -> Result<()> {
    let rows = client
        .machine_clients()
        .list(include_revoked)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&rows, fmt)?);
    Ok(())
}

/// Show one registered client.
pub async fn show_remote(
    client: &temper_client::TemperClient,
    id: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .machine_clients()
        .get(parse_uuid("machine client id", id)?)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}

/// Revoke a client. Denies authentication; grants and memberships survive (D11) —
/// which is exactly what lets `rebind` inherit reach.
pub async fn revoke_remote(
    client: &temper_client::TemperClient,
    id: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .machine_clients()
        .revoke(parse_uuid("machine client id", id)?)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&row, fmt)?);
    crate::output::hint(
        "Grants and team memberships were NOT removed — revocation denies authentication only.",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_spec_defaults_to_member() {
        assert_eq!(split_spec("acme"), ("acme".to_string(), None));
        assert_eq!(
            split_spec("acme:owner"),
            ("acme".to_string(), Some("owner".to_string()))
        );
    }

    /// A decorated ref is `sluggify(title)-<uuid>` — it contains hyphens but no colon,
    /// so splitting on the LAST colon is safe. A UUID contains no colon either.
    #[test]
    fn split_spec_does_not_mangle_decorated_refs() {
        let r = "temper-self-cognition-019f2391-e001-7933-b88a-28fb92e56ac1";
        assert_eq!(split_spec(r), (r.to_string(), None));
        assert_eq!(
            split_spec(&format!("{r}:ro")),
            (r.to_string(), Some("ro".to_string()))
        );
    }
}
