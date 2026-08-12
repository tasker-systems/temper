//! Search business logic — query embedding, cloud API dispatch, formatting.
//!
//! All testable functions. The CLI command is a thin wrapper over these.
//! Cloud-only: no local index, no manifest enrichment.

use temper_core::types::api::{SearchParams, SearchResponse};

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
    /// Cogmap refs (UUID or decorated) for single- or multi-map scope — the corpus is the union of
    /// the maps. Empty ⇒ no cogmap scope. Mutually exclusive with `context`.
    pub cogmap: &'a [String],
    pub doc_type: Option<&'a str>,
    pub limit: Option<i64>,
    /// Page offset. `/api/search` has always accepted one (`SearchParams.offset`); this door
    /// simply had no flag for it, which is what `door_coverage`'s CLI term axis recorded.
    pub offset: Option<i64>,
}

/// Build a SearchParams from CLI arguments.
pub fn build_search_params(args: CliSearchArgs<'_>) -> Result<SearchParams> {
    // Mirror the server's guard (`resolve_search_anchor`): `--context` and `--cogmap` name two
    // different homes and remain mutually exclusive. Reject here rather than relying solely on the
    // server's BadRequest, so the error surfaces before any network round-trip.
    if args.context.is_some() && !args.cogmap.is_empty() {
        return Err(TemperError::BadRequest(
            "--context and --cogmap are mutually exclusive; specify at most one home".into(),
        ));
    }
    // Search scopes to ONE anchor. Several maps at once is a composition, not a repeated flag.
    if args.cogmap.len() > 1 {
        return Err(TemperError::BadRequest(
            "search scopes to a single anchor; pass at most one --cogmap".into(),
        ));
    }
    // Parse the --cogmap ref (trailing-UUID-only). Empty ⇒ no cogmap scope.
    let cogmap_ids: Option<Vec<uuid::Uuid>> = if args.cogmap.is_empty() {
        None
    } else {
        Some(
            args.cogmap
                .iter()
                .map(|r| {
                    temper_workflow::operations::parse_ref(r)
                        .map(|id| id.0)
                        .map_err(|e| TemperError::Config(format!("invalid cogmap ref: {e}")))
                })
                .collect::<Result<Vec<_>>>()?,
        )
    };
    Ok(SearchParams {
        query: Some(args.query.to_string()),
        embedding: args.embedding,
        context_ref: args.context.map(String::from),
        cogmap_id: None,
        cogmap_ids,
        doc_type: args.doc_type.map(String::from),
        limit: args.limit,
        offset: args.offset,
        ..SearchParams::default()
    })
}

/// Call the search API with full SearchParams, returning the full response envelope — ranked hits
/// plus scope-stage diagnostics (issue #360).
pub async fn search_api(
    client: &temper_client::TemperClient,
    params: SearchParams,
) -> Result<SearchResponse> {
    client
        .search()
        .search_with_params(&params)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
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
    fn the_cli_offset_reaches_search_params() {
        // `temper search` could only ever read page 1. `SearchParams.offset` existed the whole
        // time — this asserts the CLI now fills it, which is the only half that was missing.
        let params = build_search_params(CliSearchArgs {
            query: "anything",
            embedding: None,
            context: None,
            cogmap: &[],
            doc_type: None,
            limit: Some(10),
            offset: Some(20),
        })
        .expect("a query with no anchor conflict builds");
        assert_eq!(params.offset, Some(20));
        assert_eq!(
            params.limit,
            Some(10),
            "limit must not be displaced by the new field"
        );
    }

    #[test]
    fn build_search_params_carries_the_anchor_and_nothing_else() {
        let args = CliSearchArgs {
            query: "hello",
            embedding: None,
            context: Some("temper"),
            cogmap: &[],
            doc_type: None,
            limit: Some(5),
            offset: None,
        };
        let params = build_search_params(args).expect("build_search_params");
        assert_eq!(params.query.as_deref(), Some("hello"));
        assert_eq!(params.context_ref.as_deref(), Some("temper"));
        assert_eq!(params.limit, Some(5));
    }

    #[test]
    fn build_search_params_rejects_more_than_one_anchor() {
        let two = [
            "a-019d1d24-2000-7379-8f26-ae4ae87bc5c6".to_string(),
            "b-019d1d24-2000-7379-8f26-ae4ae87bc5c7".to_string(),
        ];
        let args = CliSearchArgs {
            query: "x",
            embedding: None,
            context: None,
            cogmap: &two,
            doc_type: None,
            limit: None,
            offset: None,
        };
        let err = build_search_params(args).expect_err("two anchors must be rejected");
        assert!(
            err.to_string().contains("single anchor"),
            "the error names the one-anchor rule; got {err}"
        );
    }

    /// A [`ResourceView`] to hang a hit on — the wrapped half, identical for both arms.
    ///
    /// Built as the typed struct rather than a `serde_json::json!` literal: the hits are typed now,
    /// and a hand-written JSON stand-in is exactly the thing that used to let the CLI's rendered
    /// shape drift from the wire shape without any test noticing.
    fn sample_resource() -> temper_core::types::resource_view::ResourceView {
        use chrono::{DateTime, Utc};
        use temper_core::types::ids::{ProfileId, ResourceId};
        use temper_core::types::managed_meta::ManagedMeta;
        use temper_core::types::resource_view::ResourceView;

        let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("epoch");
        ResourceView {
            id: ResourceId::from(uuid::Uuid::nil()),
            r#ref: String::new(),
            title: "Some Title".to_string(),
            origin_uri: "test://some-title".to_string(),
            kb_context_id: None,
            context_name: None,
            context_slug: None,
            context_owner_ref: None,
            context_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            doc_type_name: "research".to_string(),
            owner_handle: "someone".to_string(),
            owner_profile_id: ProfileId::from(uuid::Uuid::nil()),
            originator_profile_id: ProfileId::from(uuid::Uuid::nil()),
            is_active: true,
            created: epoch,
            updated: epoch,
            body_hash: None,
            ingest_state: None,
            body_storage: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            content: None,
        }
        .with_derived_refs()
    }

    #[test]
    fn render_search_results_json_carries_both_arms_and_no_merged_list() {
        use temper_core::types::api::{ExactHit, SearchScope, SearchScopeInfo, WideHit};
        let exact = vec![ExactHit {
            resource: sample_resource(),
            fts_norm: 0.5,
        }];
        let wide = vec![WideHit {
            resource: sample_resource(),
            vec_norm: 0.8,
        }];
        let doc = crate::commands::search_cmd::SearchResultsResponse {
            exact,
            wide,
            scope: SearchScopeInfo {
                kind: SearchScope::Global,
                size: None,
            },
        };
        let out =
            crate::format::render(&doc, crate::format::OutputFormat::Json).expect("json render");

        let parsed: serde_json::Value = serde_json::from_str(&out).expect("single json document");
        assert!(parsed["exact"].is_array(), "exact must be an array: {out}");
        assert!(parsed["wide"].is_array(), "wide must be an array: {out}");
        assert!(
            parsed.get("results").is_none(),
            "there is no merged `results` list to render: {out}"
        );
        assert_eq!(parsed["exact"][0]["fts_norm"], 0.5);
        assert_eq!(parsed["wide"][0]["vec_norm"], 0.8);

        // The quantity is on the hit; the resource it wraps is the shared shape and carries the
        // `ref` the CLI used to inject at render time.
        assert_eq!(parsed["exact"][0]["resource"]["title"], "Some Title");
        assert_eq!(
            parsed["exact"][0]["resource"]["ref"],
            "some-title-00000000-0000-0000-0000-000000000000",
            "the server-derived `ref` rides through render untouched: {out}"
        );
        assert!(
            parsed["exact"][0]["resource"].get("fts_norm").is_none(),
            "the arm's quantity stays on the hit, never on the resource: {out}"
        );
    }
}
