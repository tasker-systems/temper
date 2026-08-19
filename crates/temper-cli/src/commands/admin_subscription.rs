//! `temper admin subscription` — operator-only subscription management.
//!
//! Thin commands: parse, resolve refs to ids, call the client, render. Mirrors
//! `admin_connection.rs`.

use temper_core::types::subscription::{CreateSubscriptionRequest, SubscriptionSelector};

use crate::error::{Result, TemperError};
use crate::format::OutputFormat;

fn parse_uuid(what: &str, raw: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(raw).map_err(|e| TemperError::Api(format!("invalid {what} '{raw}': {e}")))
}

/// Parse a selector from a JSON string or `@file.json` path.
fn parse_selector(raw: &str) -> Result<SubscriptionSelector> {
    let json_str = if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path)
            .map_err(|e| TemperError::Api(format!("failed to read selector file '{path}': {e}")))?
    } else {
        raw.to_string()
    };
    serde_json::from_str(&json_str).map_err(|e| {
        TemperError::Api(format!(
            "failed to parse selector as SubscriptionSelector JSON: {e}"
        ))
    })
}

/// Create a subscription. The two-leg authz gate runs server-side.
pub async fn create_remote(
    client: &temper_client::TemperClient,
    subscriber_table: &str,
    subscriber_id: &str,
    authoring_team_id: &str,
    connection_id: &str,
    selector: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let req = CreateSubscriptionRequest {
        subscriber_table: subscriber_table.to_string(),
        subscriber_id: parse_uuid("subscriber id", subscriber_id)?,
        authoring_team_id: parse_uuid("authoring team id", authoring_team_id)?,
        connection_id: parse_uuid("connection id", connection_id)?,
        selector: parse_selector(selector)?,
    };
    let row = client
        .subscriptions()
        .create(&req)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;

    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}

/// Enumerate subscriptions visible to the caller.
pub async fn list_remote(
    client: &temper_client::TemperClient,
    include_revoked: bool,
    connection_id: Option<&str>,
    fmt: OutputFormat,
) -> Result<()> {
    let conn_id = match connection_id {
        Some(c) => Some(parse_uuid("connection id", c)?),
        None => None,
    };
    let rows = client
        .subscriptions()
        .list(include_revoked, conn_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&rows, fmt)?);
    Ok(())
}

/// Show one subscription.
pub async fn show_remote(
    client: &temper_client::TemperClient,
    id: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .subscriptions()
        .get(parse_uuid("subscription id", id)?)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}

/// Revoke a subscription. Rows are never deleted — a revoked subscription stops matching
/// but stays resolvable for the delivery row's research-corpus property.
pub async fn revoke_remote(
    client: &temper_client::TemperClient,
    id: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .subscriptions()
        .revoke(parse_uuid("subscription id", id)?)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
}
