//! Typed sub-client for the `/api/invocations` endpoints.
//!
//! The agent-invocation envelope: `open` mints an accountability envelope (returns
//! its id), `close` terminates it with a disposition + opaque outcome, and
//! `show`/`list` read the envelope projections. Cogmap/invocation ids are substrate
//! UUIDs, not resource refs.

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::invocation::{InvocationSummary, InvocationView};
use temper_core::types::invocation_requests::{
    CloseInvocationRequest, InvocationAck, OpenInvocationRequest,
};
use temper_core::types::query_params::InvocationListQuery;

/// Sub-client for invocation-envelope operations.
pub struct InvocationsClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for InvocationsClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvocationsClient").finish_non_exhaustive()
    }
}

impl<'a> InvocationsClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/invocations — open an invocation envelope. Returns the minted id.
    pub async fn open(&self, req: &OpenInvocationRequest) -> Result<InvocationAck> {
        let token = self.http.resolve_token()?;
        let op = &ops::OPEN;
        let path = op.path(&[]);
        let req_builder = self.http.request(op, &path).json(req);
        self.http
            .send_json(&op.method(), &path, req_builder, Some(&token))
            .await
    }

    /// POST /api/invocations/{id}/close — terminate an open envelope. Returns
    /// **204 No Content**, so there is no body to deserialize.
    pub async fn close(&self, invocation_id: Uuid, req: &CloseInvocationRequest) -> Result<()> {
        let token = self.http.resolve_token()?;
        let op = &ops::CLOSE;
        let path = op.path(&[&invocation_id]);
        let req_builder = self.http.request(op, &path).json(req);
        self.http
            .send(&op.method(), &path, req_builder, Some(&token))
            .await?;
        Ok(())
    }

    /// GET /api/invocations/{id} — read one envelope plus its acts.
    pub async fn show(&self, invocation_id: Uuid) -> Result<InvocationView> {
        let token = self.http.resolve_token()?;
        let op = &ops::SHOW;
        let path = op.path(&[&invocation_id]);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/invocations[?cogmap=&status=] — list envelopes, optionally
    /// narrowed by originating cogmap and/or lifecycle status.
    pub async fn list(
        &self,
        cogmap: Option<Uuid>,
        status: Option<String>,
    ) -> Result<Vec<InvocationSummary>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_INVOCATIONS;
        let path = op.path(&[]);
        let req = self
            .http
            .request(op, &path)
            .query(&InvocationListQuery { cogmap, status });
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}
