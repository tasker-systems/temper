//! Typed sub-client for the `/api/ingest` endpoint.
//!
//! Sends a fully-processed payload (content + chunks + embeddings) as JSON.
//! The CLI handles extract → chunk → embed locally.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::ingest::{
    AppendBlockPayload, BlocksResponse, FinalizePayload, IngestPayload, SegmentedBeginResponse,
};
use temper_workflow::types::resource::ResourceRow;

/// Sub-client for ingest operations.
pub struct IngestClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for IngestClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IngestClient").finish_non_exhaustive()
    }
}

impl<'a> IngestClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/ingest — create resource with pre-processed chunks.
    pub async fn create(&self, payload: &IngestPayload) -> Result<ResourceRow> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/ingest").json(payload);
        self.http
            .send_json(&Method::POST, "/api/ingest", req, Some(&token))
            .await
    }

    /// PUT /api/ingest/:id — update resource content with new chunks.
    pub async fn update(&self, id: Uuid, payload: &IngestPayload) -> Result<ResourceRow> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/ingest/{id}");
        let req = self.http.put(&path).json(payload);
        self.http
            .send_json(&Method::PUT, &path, req, Some(&token))
            .await
    }

    /// POST /api/ingest — begin a segmented (multi-block) ingest. `payload.segmented` must be
    /// `Some`; the handler returns the segmented-begin shape (block 0 landed) instead of the
    /// one-shot `ResourceRow`.
    pub async fn begin_segmented(&self, payload: &IngestPayload) -> Result<SegmentedBeginResponse> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/ingest").json(payload);
        self.http
            .send_json(&Method::POST, "/api/ingest", req, Some(&token))
            .await
    }

    /// POST /api/resources/:id/blocks — append one already-chunked segment to a resource whose
    /// block 0 already landed. Idempotent server-side on `(resource, seq, block merkle)`.
    pub async fn append_block(
        &self,
        resource_id: Uuid,
        payload: &AppendBlockPayload,
    ) -> Result<BlocksResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/blocks");
        let req = self.http.post(&path).json(payload);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// POST /api/resources/:id/finalize — declare a segmented ingest complete. The handler
    /// responds `204 No Content` on success; there is no JSON body to decode.
    pub async fn finalize(&self, resource_id: Uuid, payload: &FinalizePayload) -> Result<()> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/finalize");
        let req = self.http.post(&path).json(payload);
        self.http
            .send(&Method::POST, &path, req, Some(&token))
            .await?;
        Ok(())
    }

    /// GET /api/resources/:id/blocks — the currently landed segment set (resume/progress read).
    pub async fn list_blocks(&self, resource_id: Uuid) -> Result<BlocksResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/blocks");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }
}
