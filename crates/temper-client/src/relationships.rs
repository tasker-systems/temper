//! Typed sub-client for the `/api/relationships` write endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::relationship_requests::{
    AssertRelationshipRequest, FoldRelationshipRequest, RelationshipAck, RetypeRelationshipRequest,
    ReweightRelationshipRequest,
};

/// Sub-client for relationship assert/retype/reweight/fold operations.
pub struct RelationshipClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for RelationshipClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelationshipClient").finish_non_exhaustive()
    }
}

impl<'a> RelationshipClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/relationships — assert a new relationship.
    pub async fn assert(&self, request: &AssertRelationshipRequest) -> Result<RelationshipAck> {
        let token = self.http.resolve_token()?;
        let path = "/api/relationships";
        let req = self.http.post(path).json(request);
        self.http
            .send_json(&Method::POST, path, req, Some(&token))
            .await
    }

    /// POST /api/relationships/{edge_handle}/retype — change kind/polarity.
    pub async fn retype(
        &self,
        edge_handle: Uuid,
        request: &RetypeRelationshipRequest,
    ) -> Result<RelationshipAck> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/relationships/{edge_handle}/retype");
        let req = self.http.post(&path).json(request);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// POST /api/relationships/{edge_handle}/reweight — change weight.
    pub async fn reweight(
        &self,
        edge_handle: Uuid,
        request: &ReweightRelationshipRequest,
    ) -> Result<RelationshipAck> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/relationships/{edge_handle}/reweight");
        let req = self.http.post(&path).json(request);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// POST /api/relationships/{edge_handle}/fold — retract.
    pub async fn fold(
        &self,
        edge_handle: Uuid,
        request: &FoldRelationshipRequest,
    ) -> Result<RelationshipAck> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/relationships/{edge_handle}/fold");
        let req = self.http.post(&path).json(request);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }
}
