//! `temper search` — thin CLI wrapper over actions::search (cloud-only).

use crate::actions::{runtime, search as search_actions};
use crate::error::Result;
use crate::format::OutputFormat;

/// Run a search. `args` carries the CLI-derived query/filter/graph fields
/// (including the already-resolved query `embedding`); the caller builds it
/// — see `main.rs`'s `Commands::Search` arm.
pub fn run(args: search_actions::CliSearchArgs<'_>, fmt: OutputFormat) -> Result<()> {
    let results = runtime::with_client(|client| {
        // The closure may borrow `args` (with_client is FnOnce, but the
        // owned Vec fields are cloned per the original construction so the
        // moved-into-future `params` owns its data).
        let params = search_actions::build_search_params(search_actions::CliSearchArgs {
            query: args.query,
            embedding: args.embedding.clone(),
            context: args.context,
            doc_type: args.doc_type,
            limit: args.limit,
            seed_ids: args.seed_ids.clone(),
            edge_types: args.edge_types.clone(),
            depth: args.depth,
            no_graph: args.no_graph,
        });
        Box::pin(async move { search_actions::search_api(client, params).await })
    })?;

    // Identity-out: every printed search row carries its decorated `ref`
    // (read from `resource_id` for search rows).
    let mut results_value = serde_json::to_value(&results)
        .map_err(|e| crate::error::TemperError::Api(format!("search serialize: {e}")))?;
    if let Some(arr) = results_value.as_array_mut() {
        for row in arr.iter_mut() {
            crate::commands::resource::inject_ref(row);
        }
    }
    let rendered = crate::format::render(&results_value, fmt)?;
    crate::output::plain(rendered);

    Ok(())
}
