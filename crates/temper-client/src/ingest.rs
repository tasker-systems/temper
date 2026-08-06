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
use temper_core::types::resource_view::ResourceView;

/// HTTP header mirroring a keyed write's `idempotency_key` (issue #581, spike rung 3-C).
///
/// The server reads the key from the request *body* (`IngestPayload.idempotency_key`); this header
/// is a transport-layer hint carrying the same value so the apex reverse proxy (`proxy.ts`) can
/// recognise a write as retry-safe without parsing the body. The two always agree — both are the
/// client-minted key. Named per the widely-used `Idempotency-Key` convention.
pub const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";

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
    ///
    /// A keyed create (`payload.idempotency_key.is_some()`) is dispatched as an idempotent write:
    /// the transient-failure retry loop replays it (the server dedups on `(owner, key)`), and the
    /// key is mirrored into an `Idempotency-Key` header so the apex proxy can retry it too. An
    /// unkeyed create keeps the safe-method-only retry policy.
    pub async fn create(&self, payload: &IngestPayload) -> Result<ResourceView> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/ingest").json(payload);
        match payload.idempotency_key {
            Some(key) => {
                let req = req.header(IDEMPOTENCY_KEY_HEADER, key.to_string());
                self.http
                    .send_json_idempotent(&Method::POST, "/api/ingest", req, Some(&token))
                    .await
            }
            None => {
                self.http
                    .send_json(&Method::POST, "/api/ingest", req, Some(&token))
                    .await
            }
        }
    }

    /// PUT /api/ingest/:id — update resource content with new chunks.
    pub async fn update(&self, id: Uuid, payload: &IngestPayload) -> Result<ResourceView> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/ingest/{id}");
        let req = self.http.put(&path).json(payload);
        self.http
            .send_json(&Method::PUT, &path, req, Some(&token))
            .await
    }

    /// POST /api/ingest — begin a segmented (multi-block) ingest. `payload.segmented` must be
    /// `Some`; the handler returns the segmented-begin shape (block 0 landed) instead of the
    /// one-shot `ResourceView`.
    pub async fn begin_segmented(&self, payload: &IngestPayload) -> Result<SegmentedBeginResponse> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/ingest").json(payload);
        // A keyed begin is idempotent-retryable exactly like a keyed one-shot create: block 0 lands
        // through the same `create_resource_impl` claim, so a replayed begin converges on the
        // committed resource (returning its landed block set) instead of minting a twin. See
        // `create` for the header/retry rationale.
        match payload.idempotency_key {
            Some(key) => {
                let req = req.header(IDEMPOTENCY_KEY_HEADER, key.to_string());
                self.http
                    .send_json_idempotent(&Method::POST, "/api/ingest", req, Some(&token))
                    .await
            }
            None => {
                self.http
                    .send_json(&Method::POST, "/api/ingest", req, Some(&token))
                    .await
            }
        }
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
