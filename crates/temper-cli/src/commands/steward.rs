//! `temper steward` — surface commands for the team-self-cognition steward's
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

/// `temper steward advance-watermark <cogmap> [event] [--boundary-fingerprint FP]`.
///
/// An omitted event is a boundary-only advance, not a mistake — see the `AdvanceWatermark` clap doc.
/// A *supplied* event that does not parse is still an error; "optional" applies to absence only.
pub fn advance_watermark(
    cogmap_ref: &str,
    event_ref: Option<&str>,
    boundary_fingerprint: Option<String>,
    fmt: OutputFormat,
) -> Result<()> {
    let cogmap = temper_workflow::operations::parse_ref(cogmap_ref)?.0;
    let event_id = event_ref
        .map(|r| {
            temper_workflow::operations::parse_ref(r)
                .map(|parsed| parsed.0)
                .map_err(|e| TemperError::Config(format!("invalid event id: {e}")))
        })
        .transpose()?;

    let ack = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            crate::actions::steward::advance_watermark_api(
                client,
                cogmap,
                event_id,
                boundary_fingerprint,
            )
            .await
        })
    })?;

    let rendered = crate::format::render(&ack, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper steward sweep [--threshold N]` — steward drift across the maps you can steward.
pub fn sweep(threshold: Option<i64>, fmt: OutputFormat) -> Result<()> {
    let query = temper_core::types::query_params::DeltaQuery { threshold };
    crate::actions::runtime::render_read(fmt, move |client| {
        Box::pin(async move { client.steward().sweep(&query).await })
    })
}

/// `temper steward candidates` — the maps you may steward.
pub fn candidates(fmt: OutputFormat) -> Result<()> {
    crate::actions::runtime::render_read(fmt, move |client| {
        Box::pin(async move { client.steward().candidates().await })
    })
}

/// `temper steward dispatch [--threshold N] [--cap N] [--correlation-id ID]` — claim one tick's
/// drifted maps.
pub fn dispatch(
    threshold: Option<i64>,
    cap: Option<i64>,
    correlation_id: Option<uuid::Uuid>,
    fmt: OutputFormat,
) -> Result<()> {
    let request = temper_core::types::steward::DispatchTickRequest { threshold, cap };
    crate::actions::runtime::render_read(fmt, move |client| {
        Box::pin(async move { client.steward().dispatch(&request, correlation_id).await })
    })
}
