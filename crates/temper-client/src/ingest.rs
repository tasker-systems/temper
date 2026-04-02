//! Typed sub-client for the `/api/ingest` endpoint.
//!
//! Sends a fully-processed payload (content + chunks + embeddings) as JSON.
//! The CLI handles extract → chunk → embed locally.

use reqwest::Method;
use uuid::Uuid;

use crate::auth;
use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::ingest::IngestPayload;
use temper_core::types::resource::ResourceRow;

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
        let token = auth::current_token()?;
        let req = self.http.post("/api/ingest").json(payload);
        self.http
            .send_json(&Method::POST, "/api/ingest", req, Some(&token))
            .await
    }

    /// PUT /api/ingest/:id — update resource content with new chunks.
    pub async fn update(&self, id: Uuid, payload: &IngestPayload) -> Result<ResourceRow> {
        let token = auth::current_token()?;
        let path = format!("/api/ingest/{id}");
        let req = self.http.put(&path).json(payload);
        self.http
            .send_json(&Method::PUT, &path, req, Some(&token))
            .await
    }
}
