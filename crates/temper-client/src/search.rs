//! Typed sub-client for the `/api/search` endpoint.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::{
    SearchDiagnostics, SearchParams, SearchResponse, UnifiedSearchResultRow,
};

/// Additive response header carrying scope-stage diagnostics (issue #360). Absent on servers old
/// enough to predate it — the client treats that as `diagnostics: None`, never an error.
const SEARCH_DIAGNOSTICS_HEADER: &str = "x-temper-search-diagnostics";

/// Sub-client for search operations.
pub struct SearchClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for SearchClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchClient").finish_non_exhaustive()
    }
}

impl<'a> SearchClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Run a vector search with a pre-computed embedding.
    pub async fn query(
        &self,
        embedding: Vec<f32>,
        context_ref: Option<String>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<UnifiedSearchResultRow>> {
        self.search(None, Some(embedding), context_ref, doc_type, limit)
            .await
    }

    /// Run a full-text search with a plain text query (no embedding needed).
    pub async fn text_query(
        &self,
        query: &str,
        context_ref: Option<String>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<UnifiedSearchResultRow>> {
        self.search(Some(query.to_string()), None, context_ref, doc_type, limit)
            .await
    }

    /// Run a unified search with optional text query and/or embedding.
    ///
    /// A convenience wrapper that discards scope-stage diagnostics and returns only the ranked
    /// rows — callers who need the diagnostics envelope (issue #360) use
    /// [`search_with_params`](Self::search_with_params).
    pub async fn search(
        &self,
        query: Option<String>,
        embedding: Option<Vec<f32>>,
        context_ref: Option<String>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<UnifiedSearchResultRow>> {
        let params = SearchParams {
            query,
            embedding,
            context_ref,
            doc_type,
            limit,
            ..SearchParams::default()
        };
        self.search_with_params(&params).await.map(|r| r.results)
    }

    /// Run a search with full control over all parameters, returning the ranked hits plus scope-stage
    /// [`SearchDiagnostics`] reassembled from the additive `x-temper-search-diagnostics` header
    /// (issue #360). The body is a bare array (unchanged contract); a server that does not emit the
    /// header yields `diagnostics: None`, never an error.
    pub async fn search_with_params(&self, params: &SearchParams) -> Result<SearchResponse> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/search").json(params);
        let resp = self
            .http
            .send(&Method::POST, "/api/search", req, Some(&token))
            .await?;

        // Parse the diagnostics header before consuming the body. `from_utf8` (not `to_str`) so a
        // hint with non-ASCII (em dashes) decodes; any parse miss degrades to `None`.
        let diagnostics = resp
            .headers()
            .get(SEARCH_DIAGNOSTICS_HEADER)
            .and_then(|v| std::str::from_utf8(v.as_bytes()).ok())
            .and_then(|s| serde_json::from_str::<SearchDiagnostics>(s).ok());

        let bytes = resp.bytes().await?;
        let results: Vec<UnifiedSearchResultRow> = serde_json::from_slice(&bytes)?;
        Ok(SearchResponse {
            results,
            diagnostics,
        })
    }
}
