//! Typed sub-client for the `/api/resources/{id}/artifacts` endpoints.

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
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
        let op = &ops::LIST_ARTIFACTS;
        let path = op.path(&[&resource_id])?;
        let req = self.http.request(op, &path).query(params);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Get a single artifact by ID under its owning resource.
    pub async fn get(&self, resource_id: Uuid, artifact_id: Uuid) -> Result<ArtifactView> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_ARTIFACT;
        let path = op.path(&[&resource_id, &artifact_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Get a single artifact by ID alone — the flat read (`GET /api/data-artifacts/{id}`).
    ///
    /// Visibility-gated on the artifact's actual owning resource; answers folded
    /// artifacts; 404 when the artifact does not exist or is not visible. The MCP
    /// `get_data_artifact` tool's door — the tool carries no `resource_id` field.
    pub async fn get_by_id(&self, artifact_id: Uuid) -> Result<ArtifactView> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_ARTIFACT_BY_ID;
        let path = op.path(&[&artifact_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Commit one data artifact to a resource.
    pub async fn commit(
        &self,
        resource_id: Uuid,
        request: &ArtifactCommitRequest,
    ) -> Result<ArtifactCommitResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::COMMIT_ARTIFACT;
        let path = op.path(&[&resource_id])?;
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// List live shapes declared for a context home.
    pub async fn list_shapes(&self, context_id: Uuid) -> Result<Vec<ShapeView>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_SHAPES;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Get a single shape by ID.
    pub async fn get_shape(&self, shape_id: Uuid) -> Result<ShapeView> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_SHAPE;
        let path = op.path(&[&shape_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Declare a shape for a data-artifact family within a context home.
    pub async fn declare_shape(
        &self,
        context_id: Uuid,
        request: &ShapeDeclareRequest,
    ) -> Result<ShapeView> {
        let token = self.http.resolve_token()?;
        let op = &ops::DECLARE_SHAPE;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// List live shapes declared for a cognitive-map home — the cogmap-home arm of
    /// [`Self::list_shapes`] (`GET /api/cognitive-maps/{id}/shapes`).
    pub async fn list_cogmap_shapes(&self, cogmap_id: Uuid) -> Result<Vec<ShapeView>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_COGMAP_SHAPES;
        let path = op.path(&[&cogmap_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Declare a shape for a data-artifact family within a cognitive-map home — the
    /// cogmap-home arm of [`Self::declare_shape`] (`POST /api/cognitive-maps/{id}/shapes`).
    pub async fn declare_cogmap_shape(
        &self,
        cogmap_id: Uuid,
        request: &ShapeDeclareRequest,
    ) -> Result<ShapeView> {
        let token = self.http.resolve_token()?;
        let op = &ops::DECLARE_COGMAP_SHAPE;
        let path = op.path(&[&cogmap_id])?;
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// The home-anchored shapes read, dispatched by the anchor's kind — the
    /// parity door for MCP's home-type-keyed tool (its `home_type` is a
    /// vocabulary, not a route).
    pub async fn list_for(
        &self,
        home: temper_core::types::home::HomeAnchor,
    ) -> Result<Vec<ShapeView>> {
        match home {
            temper_core::types::home::HomeAnchor::Context(id) => self.list_shapes(id.uuid()).await,
            temper_core::types::home::HomeAnchor::Cogmap(id) => {
                self.list_cogmap_shapes(id.uuid()).await
            }
        }
    }

    /// The home-anchored shape declare, dispatched by the anchor's kind — same
    /// door as [`Self::list_for`].
    pub async fn declare_for(
        &self,
        home: temper_core::types::home::HomeAnchor,
        request: &ShapeDeclareRequest,
    ) -> Result<ShapeView> {
        match home {
            temper_core::types::home::HomeAnchor::Context(id) => {
                self.declare_shape(id.uuid(), request).await
            }
            temper_core::types::home::HomeAnchor::Cogmap(id) => {
                self.declare_cogmap_shape(id.uuid(), request).await
            }
        }
    }
}
