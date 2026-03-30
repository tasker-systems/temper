//! Typed sub-client for the `/api/search` endpoint.

use crate::auth;
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

    /// Run a full-text or semantic search query.
    pub async fn query(&self, params: &SearchParams) -> Result<Vec<SearchResultRow>> {
        let token = auth::current_token()?;
        let req = self.http.get("/api/search").query(params);
        self.http.send_json(req, Some(&token)).await
    }
}
