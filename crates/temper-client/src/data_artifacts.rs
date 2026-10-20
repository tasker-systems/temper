//! Typed sub-client for the `/api/resources/{id}/artifacts` endpoints.

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactCommitResponse, ArtifactListParams, ArtifactPage,
    ArtifactPageRequest, ArtifactView, ARTIFACT_DRAIN_PAGE_ROWS,
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

    /// List artifacts for a resource, in whatever shape the params ask for.
    ///
    /// Returns `serde_json::Value` because the endpoint answers one of three shapes, decided
    /// server-side from the params: a bare array of every artifact (no `limit`), an
    /// `{ artifacts, next }` page (`limit` set), or `Vec<ArtifactCountRow>` (`counts=true`).
    ///
    /// This is the raw door, and its no-`limit` form is the unbounded one a later release
    /// retires. A caller that wants artifacts uses [`Self::list_page`] or [`Self::list_all`];
    /// this one remains for `counts=true`, which is not paged.
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
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// One bounded page of a resource's artifacts.
    ///
    /// `params` carries the filters (`kind`, `intent`, `include_folded`); the bound and the
    /// continuation position come from `request`, which replaces any `limit` or `after_*` on
    /// `params`. Always asks for artifacts, never counts. The server refuses a `limit` of 0 or
    /// above `MAX_ARTIFACT_PAGE_ROWS`.
    pub async fn list_page(
        &self,
        resource_id: Uuid,
        params: &ArtifactListParams,
        request: ArtifactPageRequest,
    ) -> Result<ArtifactPage> {
        let paged = ArtifactListParams {
            counts: None,
            limit: Some(request.limit),
            after_created: request.after.map(|c| c.created),
            after_id: request.after.map(|c| c.artifact_id.uuid()),
            ..params.clone()
        };
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_ARTIFACTS;
        let path = op.path(&[&resource_id])?;
        let req = self.http.request(op, &path).query(&paged);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// Every artifact of a resource, fetched a bounded page at a time.
    ///
    /// For a caller whose contract is "everything": it follows `next` until the last page
    /// (`next: null`), so no single response is unbounded and none is silently truncated. The
    /// page size is [`ARTIFACT_DRAIN_PAGE_ROWS`]. `params.limit` and `after_*` are replaced by
    /// the drain's own.
    pub async fn list_all(
        &self,
        resource_id: Uuid,
        params: &ArtifactListParams,
    ) -> Result<Vec<ArtifactView>> {
        let mut artifacts = Vec::new();
        let mut after = None;
        loop {
            let page = self
                .list_page(
                    resource_id,
                    params,
                    ArtifactPageRequest {
                        limit: ARTIFACT_DRAIN_PAGE_ROWS,
                        after,
                    },
                )
                .await?;
            artifacts.extend(page.artifacts);
            match page.next {
                Some(cursor) => after = Some(cursor),
                None => return Ok(artifacts),
            }
        }
    }

    /// Get a single artifact by ID under its owning resource.
    pub async fn get(&self, resource_id: Uuid, artifact_id: Uuid) -> Result<ArtifactView> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_ARTIFACT;
        let path = op.path(&[&resource_id, &artifact_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
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
            .send_json(&op.method(), &path, req, token.as_deref())
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
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// List live shapes declared for a context home.
    pub async fn list_shapes(&self, context_id: Uuid) -> Result<Vec<ShapeView>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_SHAPES;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// Get a single shape by ID.
    pub async fn get_shape(&self, shape_id: Uuid) -> Result<ShapeView> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_SHAPE;
        let path = op.path(&[&shape_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
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
            .send_json(&op.method(), &path, req, token.as_deref())
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
            .send_json(&op.method(), &path, req, token.as_deref())
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
            .send_json(&op.method(), &path, req, token.as_deref())
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
