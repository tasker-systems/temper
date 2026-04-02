//! Typed sub-client for the `/api/search` endpoint.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::{SearchParams, SearchResultRow};

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

    /// Run a vector similarity search.
    pub async fn query(
        &self,
        embedding: Vec<f32>,
        context_name: Option<String>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<SearchResultRow>> {
        let token = self.http.resolve_token()?;
        let params = SearchParams {
            embedding,
            context_name,
            doc_type,
            limit,
        };
        let req = self.http.post("/api/search").json(&params);
        self.http
            .send_json(&Method::POST, "/api/search", req, Some(&token))
            .await
    }
}
