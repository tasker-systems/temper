//! `temper graph entry|traverse` subcommand dispatch.
//!
//! The CLI peer of the web graph surface's two live reads. Cloud-mode-only API reads — no
//! vault-file IO. Refs are resolved trailing-UUID-only (`parse_ref`), the same addressing the
//! rest of the CLI accepts, so a decorated `slug-<uuid>` works everywhere a bare UUID does.
//!
//! Bound checks run **before** the wire — see `actions::graph` for why the traversal refuses
//! rather than letting the service clamp.

use crate::cli::GraphCmd;
use crate::error::Result;
use crate::format::OutputFormat;
use crate::output;
use uuid::Uuid;

/// Resolve a list of caller-supplied refs to ids, preserving the caller's order.
///
/// Order is preserved deliberately: `region composition` discards the caller's ordering by
/// sorting before it truncates, and that is recorded as a defect. Neither read here truncates,
/// but there is no reason to introduce the same shape.
fn resolve_refs(refs: &[String]) -> Result<Vec<Uuid>> {
    refs.iter()
        .map(|r| temper_workflow::operations::parse_ref(r).map(|id| id.0))
        .collect()
}

pub fn run(cmd: GraphCmd, fmt: OutputFormat) -> Result<()> {
    match cmd {
        GraphCmd::Entry { r#in, k } => {
            let anchors = resolve_refs(&r#in)?;
            let entry = crate::actions::runtime::with_client(|client| {
                Box::pin(async move { crate::actions::graph::entry_api(client, &anchors, k).await })
            })?;
            let rendered = crate::format::render(&entry, fmt)?;
            output::plain(rendered);
            Ok(())
        }
        GraphCmd::Traverse { from, depth } => {
            let seeds = resolve_refs(&from)?;
            // Before the wire, not after: the response has no bounds to report a clamp in.
            crate::actions::graph::validate_traverse_bounds(seeds.len(), depth)?;
            let subgraph = crate::actions::runtime::with_client(|client| {
                Box::pin(
                    async move { crate::actions::graph::traverse_api(client, &seeds, depth).await },
                )
            })?;
            let rendered = crate::format::render(&subgraph, fmt)?;
            output::plain(rendered);
            Ok(())
        }
    }
}
