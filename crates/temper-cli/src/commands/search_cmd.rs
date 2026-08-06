//! `temper search` — thin CLI wrapper over actions::search (cloud-only).

use temper_core::types::api::SearchScopeInfo;

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
    /// The exact arm's hits, each with a `ref` injected — ordered by `fts_norm`.
    pub exact: Vec<serde_json::Value>,
    /// The wide arm's hits, likewise — ordered by `vec_norm`.
    ///
    /// Held in a separate key rather than concatenated. A single `results` array would put two
    /// incommensurable quantities in one order, which is the thing this shape exists to prevent.
    pub wide: Vec<serde_json::Value>,
    pub scope: SearchScopeInfo,
}

/// Serialize an arm's hits and inject each row's decorated `ref`. One helper for both arms — the
/// arms differ in the quantity they carry, never in how a row is addressed.
fn with_refs<T: serde::Serialize>(hits: &T) -> Result<Vec<serde_json::Value>> {
    let mut v = serde_json::to_value(hits)
        .map_err(|e| crate::error::TemperError::Api(format!("search serialize: {e}")))?;
    if let Some(arr) = v.as_array_mut() {
        for row in arr.iter_mut() {
            crate::commands::resource::inject_ref(row);
        }
    }
    Ok(match v {
        serde_json::Value::Array(rows) => rows,
        other => vec![other],
    })
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
        doc_type: args.doc_type,
        limit: args.limit,
    })?;
    let response = runtime::with_client(|client| {
        Box::pin(async move { search_actions::search_api(client, params).await })
    })?;

    // Surface each arm's hint on stderr so it reaches a human watching the terminal without
    // polluting the stdout JSON a harness parses. Both arms can have something to say at once — a
    // degraded wide arm beside an exact arm that matched nothing is exactly the case a single
    // rollup hint used to flatten into one sentence.
    for hint in [
        response.exact.hint.as_deref(),
        response.wide.hint.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        crate::output::warning(hint);
    }

    // Identity-out: every printed search row carries its decorated `ref`.
    let exact = with_refs(&response.exact.hits)?;
    let wide = with_refs(&response.wide.hits)?;

    let rendered = crate::format::render(
        &SearchResultsResponse {
            exact,
            wide,
            scope: response.scope,
        },
        fmt,
    )?;
    crate::output::plain(rendered);

    Ok(())
}
