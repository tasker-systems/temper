//! `temper search` — thin CLI wrapper over actions::search.

use crate::actions::{runtime, search as search_actions};
use crate::error::Result;
use crate::format::OutputFormat;

pub fn run(
    query: &str,
    context: Option<&str>,
    doc_type: Option<&str>,
    limit: Option<i64>,
    format: &str,
    text_only: bool,
) -> Result<()> {
    let fmt = OutputFormat::parse(format);
    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;
    let manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let results = if text_only {
        // Text-only search — no local embedding needed
        runtime::with_client(|client| {
            let query_text = query.to_string();
            let context_name = context.map(String::from);
            let doc_type = doc_type.map(String::from);
            Box::pin(async move {
                search_actions::text_query_api(client, &query_text, context_name, doc_type, limit)
                    .await
            })
        })?
    } else {
        // Embed locally, then search with embedding
        let embedding = search_actions::embed_query(query)?;
        runtime::with_client(|client| {
            let embedding = embedding.clone();
            let context_name = context.map(String::from);
            let doc_type = doc_type.map(String::from);
            Box::pin(async move {
                search_actions::query_api(client, embedding, context_name, doc_type, limit).await
            })
        })?
    };

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
