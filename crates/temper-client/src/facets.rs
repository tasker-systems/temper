//! Typed sub-client for the `/api/facets` write endpoint.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::facet_requests::{
    EdgeFacetSetRequest, EdgeFacetsResponse, FacetAck, FacetSetRequest,
};
use uuid::Uuid;

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

    /// POST /api/facets — set a facet value on a resource.
    pub async fn set(&self, request: &FacetSetRequest) -> Result<FacetAck> {
        let token = self.http.resolve_token()?;
        let path = "/api/facets";
        let req = self.http.post(path).json(request);
        self.http
            .send_json(&Method::POST, path, req, Some(&token))
            .await
    }

    /// POST /api/relationships/{edge_handle}/facets — set a facet on an EDGE.
    ///
    /// A distinct method rather than an owner argument on [`Self::set`], mirroring the two
    /// endpoints: the owner is in the path, and the server authorizes the two through different
    /// gates.
    pub async fn set_on_edge(
        &self,
        edge_handle: Uuid,
        request: &EdgeFacetSetRequest,
    ) -> Result<FacetAck> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/relationships/{edge_handle}/facets");
        let req = self.http.post(&path).json(request);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// GET /api/relationships/{edge_handle}/facets — the edge's live facets.
    pub async fn list_for_edge(&self, edge_handle: Uuid) -> Result<EdgeFacetsResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/relationships/{edge_handle}/facets");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }
}
