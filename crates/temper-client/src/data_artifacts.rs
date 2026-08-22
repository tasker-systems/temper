//! Typed sub-client for the `/api/resources/{id}/artifacts` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactCommitResponse, ArtifactListParams, ArtifactView,
};
use temper_core::types::data_artifact_shape::{ShapeDeclareRequest, ShapeView};

/// Sub-client for data artifact reads and writes.
pub struct DataArtifactsClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for DataArtifactsClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataArtifactsClient")
            .finish_non_exhaustive()
    }
}

impl<'a> DataArtifactsClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// List artifacts for a resource.
    ///
    /// Returns `serde_json::Value` because the endpoint returns either
    /// `Vec<ArtifactView>` (full hydration) or `Vec<ArtifactCountRow>` (when
    /// `counts=true`), decided server-side from the params.
    pub async fn list(
        &self,
        resource_id: Uuid,
        params: &ArtifactListParams,
    ) -> Result<serde_json::Value> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/artifacts");
        let req = self.http.get(&path).query(params);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Get a single artifact by ID under its owning resource.
    pub async fn get(&self, resource_id: Uuid, artifact_id: Uuid) -> Result<ArtifactView> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/artifacts/{artifact_id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Commit one data artifact to a resource.
    pub async fn commit(
        &self,
        resource_id: Uuid,
        request: &ArtifactCommitRequest,
    ) -> Result<ArtifactCommitResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/artifacts");
        let req = self.http.post(&path).json(request);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// List live shapes declared for a context home.
    pub async fn list_shapes(&self, context_id: Uuid) -> Result<Vec<ShapeView>> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{context_id}/shapes");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Get a single shape by ID.
    pub async fn get_shape(&self, shape_id: Uuid) -> Result<ShapeView> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/shapes/{shape_id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Declare a shape for a data-artifact family within a context home.
    pub async fn declare_shape(
        &self,
        context_id: Uuid,
        request: &ShapeDeclareRequest,
    ) -> Result<ShapeView> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{context_id}/shapes");
        let req = self.http.post(&path).json(request);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }
}
