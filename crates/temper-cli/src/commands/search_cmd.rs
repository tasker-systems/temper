//! `temper search` — thin CLI wrapper over actions::search.

use crate::actions::{runtime, search as search_actions};
use crate::error::Result;
use crate::format::OutputFormat;
use uuid::Uuid;

pub fn run(
    query: &str,
    context: Option<&str>,
    doc_type: Option<&str>,
    limit: Option<i64>,
    format: &str,
    text_only: bool,
    seed_ids: Vec<Uuid>,
    edge_types: Vec<String>,
    depth: Option<i32>,
    no_graph: bool,
) -> Result<()> {
    let fmt = OutputFormat::parse(format);
    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;
    let manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let embedding = if text_only {
        None
    } else {
        Some(search_actions::embed_query(query)?)
    };

    let results = runtime::with_client(|client| {
        let params = search_actions::build_search_params(search_actions::CliSearchArgs {
            query,
            embedding: embedding.clone(),
            context,
            doc_type,
            limit,
            seed_ids: seed_ids.clone(),
            edge_types: edge_types.clone(),
            depth,
            no_graph,
        });
        Box::pin(async move { search_actions::search_api(client, params).await })
    })?;

    let enriched = search_actions::enrich_results(results, &manifest);

    if enriched.is_empty() {
        if fmt == OutputFormat::Json {
            crate::output::plain("[]");
        } else {
            crate::output::warning("No results found.");
        }
        return Ok(());
    }

    if fmt == OutputFormat::Json {
        crate::output::plain(serde_json::to_string_pretty(&enriched)?);
    } else {
        for line in search_actions::format_text(&enriched) {
            crate::output::plain(line);
        }
    }

    Ok(())
}
