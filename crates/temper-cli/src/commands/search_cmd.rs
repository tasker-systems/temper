//! `temper search` — thin CLI wrapper over actions::search (cloud-only).

use temper_core::types::api::SearchDiagnostics;

use crate::actions::{runtime, search as search_actions};
use crate::error::Result;
use crate::format::OutputFormat;

/// Envelope for `temper search --format json`.
///
/// Search previously rendered a bare top-level array, which forced every
/// consumer to special-case it against the object every other command emits.
/// Rows stay `serde_json::Value` because `inject_ref` has already decorated
/// them with a `ref` key that is not on the wire type. `diagnostics` carries
/// the scope-stage signal (issue #360) so an agent parsing stdout JSON can
/// branch on `reason`/`scope_size` without scraping the stderr hint — an
/// additive field (search stdout was already a `{results}` object), omitted
/// entirely when a pre-#360 server did not report it.
#[derive(Debug, serde::Serialize)]
pub(crate) struct SearchResultsResponse {
    pub results: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<SearchDiagnostics>,
}

/// Run a search. `args` carries the CLI-derived query/filter/graph fields
/// (including the already-resolved query `embedding`); the caller builds it
/// — see `main.rs`'s `Commands::Search` arm.
pub fn run(args: search_actions::CliSearchArgs<'_>, fmt: OutputFormat) -> Result<()> {
    // Build params before entering with_client so parse errors propagate cleanly
    // (the closure returns a Future, not a Result, so ? cannot be used inside it).
    let params = search_actions::build_search_params(search_actions::CliSearchArgs {
        query: args.query,
        embedding: args.embedding.clone(),
        context: args.context,
        cogmap: args.cogmap,
        wayfind: args.wayfind,
        lens: args.lens,
        regions: args.regions,
        doc_type: args.doc_type,
        limit: args.limit,
        seed_ids: args.seed_ids.clone(),
        edge_types: args.edge_types.clone(),
        depth: args.depth,
        no_graph: args.no_graph,
    })?;
    let response = runtime::with_client(|client| {
        Box::pin(async move { search_actions::search_api(client, params).await })
    })?;

    // Surface the scope-stage hint on stderr (issue #360) so it reaches a human/agent watching the
    // terminal without polluting the stdout JSON that a harness parses. The machine-readable
    // diagnostics still ride the stdout envelope below.
    if let Some(hint) = response
        .diagnostics
        .as_ref()
        .and_then(|d| d.hint.as_deref())
    {
        crate::output::warning(hint);
    }

    // Identity-out: every printed search row carries its decorated `ref`
    // (read from `resource_id` for search rows).
    let mut results_value = serde_json::to_value(&response.results)
        .map_err(|e| crate::error::TemperError::Api(format!("search serialize: {e}")))?;
    if let Some(arr) = results_value.as_array_mut() {
        for row in arr.iter_mut() {
            crate::commands::resource::inject_ref(row);
        }
    }
    let results = match results_value {
        serde_json::Value::Array(rows) => rows,
        other => vec![other],
    };
    let rendered = crate::format::render(
        &SearchResultsResponse {
            results,
            diagnostics: response.diagnostics,
        },
        fmt,
    )?;
    crate::output::plain(rendered);

    Ok(())
}
