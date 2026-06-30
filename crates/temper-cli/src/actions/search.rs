//! Search business logic — query embedding, cloud API dispatch, formatting.
//!
//! All testable functions. The CLI command is a thin wrapper over these.
//! Cloud-only: no local index, no manifest enrichment.

use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};

use crate::error::{Result, TemperError};

/// Embed query text locally via temper-ingest.
#[cfg(feature = "embed")]
pub fn embed_query(text: &str) -> Result<Vec<f32>> {
    temper_ingest::embed::embed_text(text)
        .map_err(|e| TemperError::Extraction(format!("embedding failed: {e}")))
}

#[cfg(not(feature = "embed"))]
pub fn embed_query(_text: &str) -> Result<Vec<f32>> {
    Err(TemperError::Config(
        "search requires the 'embed' feature — rebuild with --features embed".into(),
    ))
}

/// CLI search arguments — bundles domain params for `build_search_params`.
#[derive(Debug)]
pub struct CliSearchArgs<'a> {
    pub query: &'a str,
    pub embedding: Option<Vec<f32>>,
    pub context: Option<&'a str>,
    /// Cogmap ref (UUID or decorated) for single-map scope. Mutually exclusive with `context`.
    pub cogmap: Option<&'a str>,
    pub doc_type: Option<&'a str>,
    pub limit: Option<i64>,
    pub seed_ids: Vec<uuid::Uuid>,
    pub edge_types: Vec<String>,
    pub depth: Option<i32>,
    pub no_graph: bool,
}

/// Build a SearchParams from CLI arguments.
pub fn build_search_params(args: CliSearchArgs<'_>) -> Result<SearchParams> {
    // Mirror the `resource create` client-side guard: `--context` and `--cogmap` are mutually
    // exclusive scopes. Reject here rather than relying solely on the server's BadRequest, so the
    // error is symmetric with create and surfaces before any network round-trip.
    if args.context.is_some() && args.cogmap.is_some() {
        return Err(TemperError::BadRequest(
            "--context and --cogmap are mutually exclusive; specify exactly one scope".into(),
        ));
    }
    let cogmap_id = args
        .cogmap
        .map(|r| {
            temper_workflow::operations::parse_ref(r)
                .map(|id| id.0)
                .map_err(|e| TemperError::Config(format!("invalid cogmap ref: {e}")))
        })
        .transpose()?;
    Ok(SearchParams {
        query: Some(args.query.to_string()),
        embedding: args.embedding,
        context_ref: args.context.map(String::from),
        cogmap_id,
        doc_type: args.doc_type.map(String::from),
        limit: args.limit,
        seed_ids: if args.seed_ids.is_empty() {
            None
        } else {
            Some(args.seed_ids)
        },
        edge_types: if args.edge_types.is_empty() {
            None
        } else {
            Some(args.edge_types)
        },
        graph_depth: args.depth,
        graph_expand: !args.no_graph,
        ..SearchParams::default()
    })
}

/// Call the search API with full SearchParams.
pub async fn search_api(
    client: &temper_client::TemperClient,
    params: SearchParams,
) -> Result<Vec<UnifiedSearchResultRow>> {
    client
        .search()
        .search_with_params(&params)
        .await
        .map_err(crate::commands::client_err)
}

/// Truncate a snippet to max_chars (character count), breaking at word boundaries.
pub fn truncate_snippet(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let byte_offset = text
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let truncated = &text[..byte_offset];
    match truncated.rfind(' ') {
        Some(pos) => format!("{}...", &text[..pos]),
        None => format!("{truncated}..."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate_snippet("short", 200), "short");
    }

    #[test]
    fn test_truncate_long() {
        let long = "word ".repeat(100);
        let result = truncate_snippet(&long, 20);
        assert!(result.ends_with("..."));
        assert!(result.len() < 30);
    }

    #[test]
    fn test_truncate_no_space() {
        assert_eq!(truncate_snippet("aaaaaaaaaaaa", 5), "aaaaa...");
    }

    #[test]
    fn test_build_search_params_passes_graph_flags() {
        let args = CliSearchArgs {
            query: "hello",
            embedding: None,
            context: Some("temper"),
            cogmap: None,
            doc_type: None,
            limit: Some(5),
            seed_ids: vec![],
            edge_types: vec!["broader".into()],
            depth: Some(3),
            no_graph: false,
        };
        let params = build_search_params(args).expect("build_search_params");
        assert_eq!(params.query.as_deref(), Some("hello"));
        assert_eq!(params.context_ref.as_deref(), Some("temper"));
        assert_eq!(params.limit, Some(5));
        assert_eq!(
            params.edge_types.as_deref(),
            Some(&["broader".to_string()][..])
        );
        assert_eq!(params.graph_depth, Some(3));
        assert!(params.graph_expand);
    }

    #[test]
    fn test_build_search_params_no_graph_disables_expand() {
        let args = CliSearchArgs {
            query: "x",
            embedding: None,
            context: None,
            cogmap: None,
            doc_type: None,
            limit: None,
            seed_ids: vec![],
            edge_types: vec![],
            depth: None,
            no_graph: true,
        };
        let params = build_search_params(args).expect("build_search_params");
        assert!(!params.graph_expand);
    }

    #[test]
    fn test_build_search_params_cogmap_uuid() {
        let id = uuid::Uuid::now_v7();
        let args = CliSearchArgs {
            query: "q",
            embedding: None,
            context: None,
            cogmap: Some(&id.to_string()),
            doc_type: None,
            limit: None,
            seed_ids: vec![],
            edge_types: vec![],
            depth: None,
            no_graph: true,
        };
        let params = build_search_params(args).expect("build_search_params");
        assert_eq!(params.cogmap_id, Some(id));
        assert!(params.context_ref.is_none());
    }

    #[test]
    fn test_build_search_params_context_and_cogmap_mutually_exclusive() {
        let id = uuid::Uuid::now_v7();
        let id_str = id.to_string();
        let args = CliSearchArgs {
            query: "q",
            embedding: None,
            context: Some("temper"),
            cogmap: Some(&id_str),
            doc_type: None,
            limit: None,
            seed_ids: vec![],
            edge_types: vec![],
            depth: None,
            no_graph: true,
        };
        let err = build_search_params(args).expect_err("both scopes must be rejected client-side");
        assert!(
            err.to_string().contains("mutually exclusive"),
            "error should name the mutual-exclusion guard; got {err}"
        );
    }

    #[test]
    fn render_search_results_json_is_passthrough_array() {
        use temper_core::types::api::UnifiedSearchResultRow;
        let rows: Vec<UnifiedSearchResultRow> = vec![UnifiedSearchResultRow {
            resource_id: uuid::Uuid::nil(),
            title: "Test Resource".to_string(),
            slug: "test-resource".to_string(),
            kb_uri: "kb://temper/test-resource".to_string(),
            origin_uri: "file:///some/path.md".to_string(),
            context: None,
            doc_type: "task".to_string(),
            fts_score: 0.5,
            vector_score: 0.0,
            graph_score: 0.0,
            combined_score: 0.5,
            origin: "fts".to_string(),
            context_slug: None,
            context_owner_ref: None,
        }];
        let out =
            crate::format::render(&rows, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.starts_with('['), "json should be an array: {out}");
        assert!(out.contains("\"slug\""), "json: {out}");
        assert!(out.contains("\"title\""), "json: {out}");
    }
}
