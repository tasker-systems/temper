//! `temper steward delta|advance-watermark` — surface commands for the team-self-cognition steward's
//! ingest trigger (T4a). Each resolves the cogmap ref → substrate UUID, dispatches one API call, and
//! renders the typed result.

use crate::error::{Result, TemperError};
use crate::format::OutputFormat;

/// `temper steward delta <cogmap> [--threshold N]`.
pub fn delta(cogmap_ref: &str, threshold: Option<i64>, fmt: OutputFormat) -> Result<()> {
    let cogmap = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let delta = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::steward::delta_api(client, cogmap, threshold).await })
    })?;

    let rendered = crate::format::render(&delta, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper steward advance-watermark <cogmap> <event>`.
pub fn advance_watermark(cogmap_ref: &str, event_ref: &str, fmt: OutputFormat) -> Result<()> {
    let cogmap = temper_workflow::operations::parse_ref(cogmap_ref)?.0;
    let event_id = temper_workflow::operations::parse_ref(event_ref)
        .map_err(|e| TemperError::Config(format!("invalid event id: {e}")))?
        .0;

    let ack = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            crate::actions::steward::advance_watermark_api(client, cogmap, event_id).await
        })
    })?;

    let rendered = crate::format::render(&ack, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}
