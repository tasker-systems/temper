//! Typed sub-client for the `/api/embed` reads the MCP binding crosses.

use std::collections::HashMap;

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::workflow_job::EmbeddingStatus;

/// Sub-client for embed-pipeline reads.
pub struct EmbedClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for EmbedClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbedClient").finish_non_exhaustive()
    }
}

impl<'a> EmbedClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// GET /api/embed/status?ids=… — the batch read the MCP resources tools derive each
    /// response's `embedding_status` from. The route is deliberately unregistered on the
    /// API's OpenAPI contract (same posture as the embed admin triggers), so this method
    /// is its only typed caller; the ids a caller sends are the ids a visibility-gated
    /// read already returned to it (the route's own contract — it does not re-gate).
    pub async fn status(&self, resource_ids: &[Uuid]) -> Result<HashMap<Uuid, EmbeddingStatus>> {
        let token = self.http.resolve_token()?;
        let ids = resource_ids
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let path = format!("/api/embed/status?ids={ids}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }
}
