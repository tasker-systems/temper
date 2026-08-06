//! Typed sub-client for the `/api/search` endpoint.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::{ExactHit, SearchParams, SearchResponse, WideHit};

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

    /// Run a vector search with a pre-computed embedding, returning the **wide** arm.
    ///
    /// Wide, not exact: an embedding is the "you have the idea, not the words" question, and the
    /// exact arm has nothing to say about it — with no `query` text it matches nothing at all.
    pub async fn query(
        &self,
        embedding: Vec<f32>,
        context_ref: Option<String>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<WideHit>> {
        let params = SearchParams {
            embedding: Some(embedding),
            context_ref,
            doc_type,
            limit,
            ..SearchParams::default()
        };
        self.search_with_params(&params).await.map(|r| r.wide.hits)
    }

    /// Run a full-text search with a plain text query, returning the **exact** arm.
    pub async fn text_query(
        &self,
        query: &str,
        context_ref: Option<String>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<ExactHit>> {
        self.search(Some(query.to_string()), None, context_ref, doc_type, limit)
            .await
    }

    /// Run a search and return the **exact** arm's hits alone.
    ///
    /// A convenience wrapper for callers who want term matching and nothing else. It discards the
    /// wide arm and both dispositions, so it can return an empty vec for a query whose wide arm
    /// answered — use [`search_with_params`](Self::search_with_params) to see both arms and why each
    /// is shaped as it is.
    pub async fn search(
        &self,
        query: Option<String>,
        embedding: Option<Vec<f32>>,
        context_ref: Option<String>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<ExactHit>> {
        let params = SearchParams {
            query,
            embedding,
            context_ref,
            doc_type,
            limit,
            ..SearchParams::default()
        };
        self.search_with_params(&params).await.map(|r| r.exact.hits)
    }

    /// Run a search with full control over all parameters, returning **both arms unmerged** plus the
    /// scope they share.
    ///
    /// Diagnostics arrive in the body. They previously rode the additive
    /// `x-temper-search-diagnostics` response header, which existed only because the body was a bare
    /// array; the body is an object now, and each arm carries its own `reason`/`hint` beside the hits
    /// it is describing. The header, its percent-encoding scar, and the old-server `None` degrade
    /// path are all gone with it.
    pub async fn search_with_params(&self, params: &SearchParams) -> Result<SearchResponse> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/search").json(params);
        let resp = self
            .http
            .send(&Method::POST, "/api/search", req, Some(&token))
            .await?;
        let bytes = resp.bytes().await?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}
