//! `temper search` — thin CLI wrapper over actions::search (cloud-only).

use crate::actions::{runtime, search as search_actions};
use crate::error::Result;
use crate::format::OutputFormat;
use uuid::Uuid;

#[expect(
    clippy::too_many_arguments,
    reason = "all args are CLI-derived primitives; bundling into a struct would mirror clap-generated fields with no semantic benefit"
)]
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

    if results.is_empty() {
        if fmt == OutputFormat::Json {
            crate::output::plain("[]");
        } else {
            crate::output::warning("No results found.");
        }
        return Ok(());
    }

    if fmt == OutputFormat::Json {
        crate::output::plain(serde_json::to_string_pretty(&results)?);
    } else {
        for line in search_actions::format_text(&results) {
            crate::output::plain(line);
        }
    }

    Ok(())
}
