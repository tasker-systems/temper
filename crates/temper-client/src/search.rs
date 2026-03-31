//! Typed sub-client for the `/api/search` endpoint.

use crate::auth;
use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::{SearchParams, SearchResultRow};
use uuid::Uuid;

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
        context: Option<Uuid>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<SearchResultRow>> {
        let token = auth::current_token()?;
        let params = SearchParams {
            embedding,
            context,
            doc_type,
            limit,
        };
        let req = self.http.post("/api/search").json(&params);
        self.http.send_json(req, Some(&token)).await
    }
}
