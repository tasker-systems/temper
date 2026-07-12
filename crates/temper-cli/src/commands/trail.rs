//! `temper trail node|edge <ref>` — read a graph element's event trail (the
//! append-only ledger history of the events that produced and mutated it).
//!
//! This is the CLI peer of the web UI's trail rail: both consume the same
//! access-gated `GET /api/graph/elements/{kind}/{id}/trail` read. The ref is
//! resolved trailing-UUID-only (`parse_ref`) — a resource ref for a node, an
//! edge UUID for an edge — and the resolved id is dispatched to the API.

use crate::cli::CliElementKind;
use crate::error::Result;
use crate::format::OutputFormat;

/// `temper trail <kind> <ref>` — render the element's event trail.
pub fn run(kind: CliElementKind, element_ref: &str, fmt: OutputFormat) -> Result<()> {
    let element_id = temper_workflow::operations::parse_ref(element_ref)?.0;
    let kind = kind.into();

    let trail = crate::actions::runtime::with_client(|client| {
        Box::pin(
            async move { crate::actions::trail::element_trail_api(client, kind, element_id).await },
        )
    })?;

    let rendered = crate::format::render(&trail, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}
