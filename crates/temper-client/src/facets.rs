//! Typed sub-client for the `/api/facets` write endpoint.

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::facet_requests::{
    EdgeFacetSetRequest, EdgeFacetsResponse, FacetAck, FacetRetractAck, FacetSetRequest,
    ResourceFacetsResponse,
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
        let op = &ops::SET_FACET;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
        let op = &ops::SET_EDGE_FACET;
        let path = op.path(&[&edge_handle])?;
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/relationships/{edge_handle}/facets — the edge's live facets.
    pub async fn list_for_edge(&self, edge_handle: Uuid) -> Result<EdgeFacetsResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_EDGE_FACETS;
        let path = op.path(&[&edge_handle])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// DELETE /api/relationships/{edge_handle}/facets/{property_id} — retract one facet row
    /// owned by the edge, addressed by the id `list_for_edge` returned.
    ///
    /// DELETE has no body, so per-act authorship (`act`) rides query params; an empty
    /// `ActInput` serializes to nothing and appends no query string.
    pub async fn retract_on_edge(
        &self,
        edge_handle: Uuid,
        property_id: Uuid,
        act: &temper_core::types::authorship::ActInput,
    ) -> Result<FacetRetractAck> {
        let token = self.http.resolve_token()?;
        let op = &ops::RETRACT_EDGE_FACET;
        let path = op.path(&[&edge_handle, &property_id])?;
        let req = self.http.request(op, &path).query(act);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/resources/{id}/facets — the resource's live facets, one row per assert.
    ///
    /// Returns every live row rather than the collapsed single value `resource show` carries in
    /// `open_meta` — including each row's weight, which that collapse discards.
    pub async fn list_for_resource(&self, resource: Uuid) -> Result<ResourceFacetsResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_RESOURCE_FACETS;
        let path = op.path(&[&resource])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}
