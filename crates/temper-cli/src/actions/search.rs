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
    /// Wayfind region-salience scope across the principal's visible maps. Mutually exclusive
    /// with `context` and `cogmap`.
    pub wayfind: bool,
    /// Lens ref (UUID or decorated) overriding wayfind region selection. Requires `wayfind`.
    pub lens: Option<&'a str>,
    /// Top-N regions to scope into for wayfind (default/ceiling are server-side). Requires `wayfind`.
    pub regions: Option<i64>,
    pub doc_type: Option<&'a str>,
    pub limit: Option<i64>,
    pub seed_ids: Vec<uuid::Uuid>,
    pub edge_types: Vec<String>,
    pub depth: Option<i32>,
    pub no_graph: bool,
}

/// Build a SearchParams from CLI arguments.
pub fn build_search_params(args: CliSearchArgs<'_>) -> Result<SearchParams> {
    // Mirror the server's guard (§6): `--context`, `--cogmap`, and `--wayfind` are three mutually
    // exclusive scope-resolution paths. Reject here rather than relying solely on the server's
    // BadRequest, so the error surfaces before any network round-trip and is symmetric with create.
    let scope_count = [args.context.is_some(), args.cogmap.is_some(), args.wayfind]
        .into_iter()
        .filter(|&set| set)
        .count();
    if scope_count > 1 {
        return Err(TemperError::BadRequest(
            "--context, --cogmap, and --wayfind are mutually exclusive; specify exactly one scope"
                .into(),
        ));
    }
    // `--lens` / `--regions` only modify the wayfind funnel; they are meaningless without it.
    if !args.wayfind && (args.lens.is_some() || args.regions.is_some()) {
        return Err(TemperError::BadRequest(
            "--lens and --regions require --wayfind".into(),
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
    let lens_id = args
        .lens
        .map(|r| {
            temper_workflow::operations::parse_ref(r)
                .map(|id| id.0)
                .map_err(|e| TemperError::Config(format!("invalid lens ref: {e}")))
        })
        .transpose()?;
    Ok(SearchParams {
        query: Some(args.query.to_string()),
        embedding: args.embedding,
        context_ref: args.context.map(String::from),
        cogmap_id,
        wayfind: args.wayfind,
        lens_id,
        regions: args.regions,
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
            wayfind: false,
            lens: None,
            regions: None,
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
            wayfind: false,
            lens: None,
            regions: None,
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
            wayfind: false,
            lens: None,
            regions: None,
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
            wayfind: false,
            lens: None,
            regions: None,
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
    fn test_build_search_params_wayfind() {
        let lens = uuid::Uuid::now_v7();
        let lens_str = lens.to_string();
        let args = CliSearchArgs {
            query: "q",
            embedding: None,
            context: None,
            cogmap: None,
            wayfind: true,
            lens: Some(&lens_str),
            regions: Some(5),
            doc_type: None,
            limit: None,
            seed_ids: vec![],
            edge_types: vec![],
            depth: None,
            no_graph: true,
        };
        let params = build_search_params(args).expect("build_search_params");
        assert!(params.wayfind);
        assert_eq!(params.lens_id, Some(lens));
        assert_eq!(params.regions, Some(5));
        assert!(params.context_ref.is_none());
        assert!(params.cogmap_id.is_none());
    }

    #[test]
    fn test_build_search_params_wayfind_context_mutually_exclusive() {
        let args = CliSearchArgs {
            query: "q",
            embedding: None,
            context: Some("temper"),
            cogmap: None,
            wayfind: true,
            lens: None,
            regions: None,
            doc_type: None,
            limit: None,
            seed_ids: vec![],
            edge_types: vec![],
            depth: None,
            no_graph: true,
        };
        let err = build_search_params(args).expect_err("context + wayfind must be rejected");
        assert!(
            err.to_string().contains("mutually exclusive"),
            "error should name the mutual-exclusion guard; got {err}"
        );
    }

    #[test]
    fn test_build_search_params_lens_requires_wayfind() {
        let lens = uuid::Uuid::now_v7();
        let lens_str = lens.to_string();
        let args = CliSearchArgs {
            query: "q",
            embedding: None,
            context: None,
            cogmap: None,
            wayfind: false,
            lens: Some(&lens_str),
            regions: None,
            doc_type: None,
            limit: None,
            seed_ids: vec![],
            edge_types: vec![],
            depth: None,
            no_graph: true,
        };
        let err = build_search_params(args).expect_err("--lens without --wayfind must be rejected");
        assert!(
            err.to_string().contains("require --wayfind"),
            "error should name the wayfind requirement; got {err}"
        );
    }

    #[test]
    fn render_search_results_json_is_object_with_results_key() {
        use temper_core::types::api::UnifiedSearchResultRow;
        let rows = [UnifiedSearchResultRow {
            resource_id: uuid::Uuid::nil(),
            slug: "some-slug".to_string(),
            title: "Some Title".to_string(),
            kb_uri: "kb://temper/some-slug".to_string(),
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
        let rows_value: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| serde_json::to_value(r).expect("row to value"))
            .collect();
        let doc = crate::commands::search_cmd::SearchResultsResponse {
            results: rows_value,
        };
        let out =
            crate::format::render(&doc, crate::format::OutputFormat::Json).expect("json render");

        assert!(out.starts_with('{'), "json should be an object: {out}");
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("single json document");
        assert!(
            parsed["results"].is_array(),
            "results must be an array: {out}"
        );
        assert!(out.contains("\"slug\""), "json: {out}");
        assert!(out.contains("\"title\""), "json: {out}");
    }
}
