//! `temper search` — thin CLI wrapper over actions::search (cloud-only).

use temper_core::types::api::{ExactHit, SearchReason, SearchScopeInfo, WideHit};
use temper_core::types::diagnostics::{Diagnostic, DiagnosticLevel};

use crate::actions::{runtime, search as search_actions};
use crate::error::Result;
use crate::format::OutputFormat;

/// Envelope for `temper search --format json`.
///
/// Search previously rendered a bare top-level array, which forced every
/// consumer to special-case it against the object every other command emits.
/// The arms carry their wire types directly rather than `serde_json::Value`.
/// They held `Value` so a render-time `inject_ref` pass could decorate each row
/// with a `ref` the wire type did not have; `ResourceView` carries its own
/// `ref` and `context_ref`, derived server-side by `with_derived_refs`, so that
/// pass had nothing left to add. Passing the typed hits through is what keeps
/// the CLI's stdout identical to the API's body instead of a re-serialization
/// that can drift from it.
///
/// `diagnostics` carries the per-arm `reason`, `degraded` flag, and `hint`
/// strings that the API wire already sends but the CLI previously stripped —
/// routing `hint` to stderr and dropping `reason`/`degraded` entirely. An
/// additive field (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`),
/// omitted entirely when every arm is `Ok` and neither is degraded.
#[derive(Debug, serde::Serialize)]
pub(crate) struct SearchResultsResponse {
    /// The exact arm's hits — ordered by `fts_norm`.
    pub exact: Vec<ExactHit>,
    /// The wide arm's hits, likewise — ordered by `vec_norm`.
    ///
    /// Held in a separate key rather than concatenated. A single `results` array would put two
    /// incommensurable quantities in one order, which is the thing this shape exists to prevent.
    pub wide: Vec<WideHit>,
    pub scope: SearchScopeInfo,
    /// Per-arm diagnostics: `reason`/`degraded`/`hint` from the API wire,
    /// surfaced so an agent parsing stdout JSON can branch on them without
    /// scraping stderr. Absent when every arm is `Ok` and neither is degraded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
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
        offset: args.offset,
        within: args.within,
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

    // Build structured diagnostics from the API's per-arm reason/hint/degraded fields.
    // These ride the stdout payload so an agent parsing JSON can branch on them
    // without scraping stderr. The stderr warnings above serve the TTY/TOON human.
    let diagnostics = build_search_diagnostics(&response);

    // Identity-out: every printed search row carries its decorated `ref` — on the `resource` it
    // wraps, filled by the server rather than injected here.
    let rendered = crate::format::render(
        &SearchResultsResponse {
            exact: response.exact.hits,
            wide: response.wide.hits,
            scope: response.scope,
            diagnostics,
        },
        fmt,
    )?;
    crate::output::plain(rendered);

    Ok(())
}

/// Translate the API's per-arm `reason`/`hint`/`degraded` into structured
/// diagnostics for the stdout payload. Each arm can contribute zero or more
/// diagnostics; the result is empty when every arm is `Ok` and neither is
/// degraded, which `skip_serializing_if = "Vec::is_empty"` then omits from the
/// wire entirely.
pub(crate) fn build_search_diagnostics(
    response: &temper_core::types::api::SearchResponse,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    // Exact arm
    if response.exact.reason != SearchReason::Ok {
        let (code, message) = match response.exact.reason {
            SearchReason::NoMatch => (
                "exact-no-match",
                "The exact arm found nothing — the scope was non-empty but nothing matched the query.",
            ),
            SearchReason::OutOfScope => (
                "exact-out-of-scope",
                "The exact arm's scope resolved to zero candidates — a different query phrasing will not help.",
            ),
            SearchReason::Ok => unreachable!(),
        };
        diags.push(Diagnostic {
            level: DiagnosticLevel::Info,
            code,
            message: message.to_string(),
            hint: response.exact.hint.clone(),
        });
    }

    // Wide arm
    if response.wide.degraded {
        diags.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            code: "wide-degraded",
            message: "The wide arm was degraded — the server could not embed the query. \
                     Vector results may be incomplete or absent."
                .to_string(),
            hint: response.wide.hint.clone(),
        });
    } else if response.wide.reason != SearchReason::Ok {
        let (code, message) = match response.wide.reason {
            SearchReason::NoMatch => (
                "wide-no-match",
                "The wide arm found nothing — the scope was non-empty but nothing matched the query.",
            ),
            SearchReason::OutOfScope => (
                "wide-out-of-scope",
                "The wide arm's scope resolved to zero candidates — a different query phrasing will not help.",
            ),
            SearchReason::Ok => unreachable!(),
        };
        diags.push(Diagnostic {
            level: DiagnosticLevel::Info,
            code,
            message: message.to_string(),
            hint: response.wide.hint.clone(),
        });
    }

    diags
}
