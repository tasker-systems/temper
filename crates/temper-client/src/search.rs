//! Typed sub-client for the `/api/search` endpoint.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};

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
    pub async fn search(
        &self,
        query: Option<String>,
        embedding: Option<Vec<f32>>,
        context_ref: Option<String>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<UnifiedSearchResultRow>> {
        let token = self.http.resolve_token()?;
        let params = SearchParams {
            query,
            embedding,
            context_ref,
            doc_type,
            limit,
            ..SearchParams::default()
        };
        let req = self.http.post("/api/search").json(&params);
        self.http
            .send_json(&Method::POST, "/api/search", req, Some(&token))
            .await
    }

    /// Run a search with full control over all parameters including graph expansion.
    pub async fn search_with_params(
        &self,
        params: &SearchParams,
    ) -> Result<Vec<UnifiedSearchResultRow>> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/search").json(params);
        self.http
            .send_json(&Method::POST, "/api/search", req, Some(&token))
            .await
    }
}
