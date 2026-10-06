//! `temper graph` subcommand dispatch.
//!
//! The CLI peer of the web graph surface's two live reads. Cloud-mode-only API reads — no
//! vault-file IO. Refs are resolved trailing-UUID-only (`parse_ref`), the same addressing the
//! rest of the CLI accepts, so a decorated `slug-<uuid>` works everywhere a bare UUID does.
//!
//! Bound checks run **before** the wire — see `actions::graph` for why the traversal refuses
//! rather than letting the service clamp.

use crate::cli::GraphCmd;
use crate::error::{Result, TemperError};
use crate::format::OutputFormat;
use crate::output;
use temper_core::types::graph::EdgeKind;
use temper_core::types::graph_atlas::SliceRequest;
use temper_core::types::query_params::{
    CogmapPanoramaQuery, ContextCompositionQuery, ContextPanoramaQuery, RegionCompositionQuery,
};
use uuid::Uuid;

/// Resolve `--from` seeds to ids, preserving the caller's order.
///
/// Seeds are **resource** refs, which always carry a trailing uuid, so this needs no server —
/// unlike `--in`, whose anchors may be `@owner/slug` context refs (see
/// `actions::graph::resolve_anchors`).
///
/// Order is preserved deliberately: `region composition` discards the caller's ordering by
/// sorting before it truncates, and that is recorded as a defect. Neither read here truncates,
/// but there is no reason to introduce the same shape.
fn resolve_seeds(refs: &[String]) -> Result<Vec<Uuid>> {
    refs.iter()
        .map(|r| temper_workflow::operations::parse_ref(r).map(|id| id.0))
        .collect()
}

pub fn run(cmd: GraphCmd, fmt: OutputFormat) -> Result<()> {
    match cmd {
        GraphCmd::Entry { r#in, k } => {
            // Anchors resolve inside the client closure: `--in` takes a context OR a cogmap ref,
            // and the `@owner/slug` context form has no uuid to parse — only the server can
            // resolve it. See `actions::graph::classify_anchor`.
            let entry = crate::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let anchors = crate::actions::graph::resolve_anchors(client, &r#in).await?;
                    crate::actions::graph::entry_api(client, &anchors, k).await
                })
            })?;
            let rendered = crate::format::render(&entry, fmt)?;
            output::plain(rendered);
            Ok(())
        }
        GraphCmd::Traverse { from, depth } => {
            let seeds = resolve_seeds(&from)?;
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
        GraphCmd::Home => crate::actions::runtime::render_read(fmt, move |client| {
            Box::pin(async move { client.graph().home().await })
        }),
        GraphCmd::ContextPanorama {
            context,
            group_by,
            container_types,
            depth,
        } => {
            let query = ContextPanoramaQuery {
                context_ref: context,
                group_by,
                container_types: csv(&container_types),
                depth,
            };
            crate::actions::runtime::render_read(fmt, move |client| {
                Box::pin(async move { client.graph().context_panorama(&query).await })
            })
        }
        GraphCmd::ContextComposition {
            context,
            container,
            group,
            container_types,
            depth,
            container_depth,
        } => {
            let container = container
                .map(|r| temper_workflow::operations::parse_ref(&r).map(|id| id.0))
                .transpose()?;
            let query = ContextCompositionQuery {
                context_ref: context,
                container,
                group,
                container_types: csv(&container_types),
                depth,
                container_depth,
            };
            crate::actions::runtime::render_read(fmt, move |client| {
                Box::pin(async move { client.graph().context_composition(&query).await })
            })
        }
        GraphCmd::RegionComposition { regions, depth } => {
            let ids = regions
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let query = RegionCompositionQuery { ids, depth };
            crate::actions::runtime::render_read(fmt, move |client| {
                Box::pin(async move { client.graph().region_composition(&query).await })
            })
        }
        GraphCmd::CogmapPanorama { cogmap, lens } => {
            let cogmap = temper_workflow::operations::parse_ref(&cogmap)?.0;
            let query = CogmapPanoramaQuery { lens_id: lens };
            crate::actions::runtime::render_read(fmt, move |client| {
                Box::pin(async move { client.graph().cogmap_panorama(cogmap, &query).await })
            })
        }
        GraphCmd::CogmapSlice {
            cogmap,
            seeds,
            depth,
            edge_kinds,
        } => {
            let cogmap = temper_workflow::operations::parse_ref(&cogmap)?.0;
            let request = SliceRequest {
                seeds: resolve_seeds(&seeds)?,
                depth,
                edge_kinds: edge_kinds
                    .iter()
                    .map(|k| parse_edge_kind(k))
                    .collect::<Result<_>>()?,
            };
            crate::actions::runtime::render_read(fmt, move |client| {
                Box::pin(async move { client.graph().cogmap_slice(cogmap, &request).await })
            })
        }
    }
}

/// A repeatable flag as the comma-separated value the door's query string takes; none is absent.
fn csv(values: &[String]) -> Option<String> {
    (!values.is_empty()).then(|| values.join(","))
}

/// An `--edge-kind` value in its wire (snake_case) spelling.
fn parse_edge_kind(kind: &str) -> Result<EdgeKind> {
    serde_json::from_value(serde_json::Value::String(kind.to_string())).map_err(|_| {
        TemperError::BadRequest(format!(
            "unknown --edge-kind {kind:?}; use the snake_case kind name, e.g. leads_to"
        ))
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::Cli;

    #[test]
    fn an_edge_kind_parses_in_its_wire_spelling() {
        assert_eq!(parse_edge_kind("leads_to").unwrap(), EdgeKind::LeadsTo);
        assert!(matches!(
            parse_edge_kind("LeadsTo"),
            Err(TemperError::BadRequest(m)) if m.contains("snake_case")
        ));
    }

    #[test]
    fn repeatable_flags_join_into_the_doors_csv() {
        assert_eq!(csv(&[]), None);
        assert_eq!(
            csv(&["goal".to_string(), "task".to_string()]).as_deref(),
            Some("goal,task")
        );
    }

    #[test]
    fn a_composition_drills_a_container_or_a_group_not_both() {
        assert!(Cli::try_parse_from([
            "temper",
            "graph",
            "context-composition",
            "@me/x",
            "--container",
            "c",
            "--group",
            "doc_type:task",
        ])
        .is_err());
    }

    #[test]
    fn region_composition_and_cogmap_slice_need_their_focus() {
        assert!(Cli::try_parse_from(["temper", "graph", "region-composition"]).is_err());
        assert!(Cli::try_parse_from(["temper", "graph", "cogmap-slice", "m"]).is_err());
    }
}
