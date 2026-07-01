//! Typed sub-client for the `/api/facets` write endpoint.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::facet_requests::{FacetAck, FacetSetRequest};

/// Sub-client for facet set operations.
pub struct FacetClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for FacetClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FacetClient").finish_non_exhaustive()
    }
}

impl<'a> FacetClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/facets — set a facet value.
    pub async fn set(&self, request: &FacetSetRequest) -> Result<FacetAck> {
        let token = self.http.resolve_token()?;
        let path = "/api/facets";
        let req = self.http.post(path).json(request);
        self.http
            .send_json(&Method::POST, path, req, Some(&token))
            .await
    }
}
