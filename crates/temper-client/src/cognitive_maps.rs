//! Typed sub-client for the `/api/cognitive-maps` endpoints.
//!
//! Reconcile is the L0 delivery & lifecycle write path: the operator embeds a committed manifest
//! client-side and PUTs a PRE-EMBEDDED desired-state request. The server diffs it against the map's
//! current `provenance: kernel` slice and applies additive-only mutations.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::reconcile::{ReconcileCogmapRequest, ReconcileOutcome};

/// Sub-client for cognitive-map operations.
pub struct CognitiveMapClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for CognitiveMapClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CognitiveMapClient").finish_non_exhaustive()
    }
}

impl<'a> CognitiveMapClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// PUT /api/cognitive-maps/{id} — reconcile a cognitive map's content to the desired manifest
    /// (admin-gated, idempotent). Returns the run outcome (`created`/`updated`/`folded`/`unchanged`).
    pub async fn reconcile_cognitive_map(
        &self,
        cogmap_id: Uuid,
        payload: &ReconcileCogmapRequest,
    ) -> Result<ReconcileOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/cognitive-maps/{cogmap_id}");
        let req = self.http.put(&path).json(payload);
        self.http
            .send_json(&Method::PUT, &path, req, Some(&token))
            .await
    }
}
