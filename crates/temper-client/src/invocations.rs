//! Typed sub-client for the `/api/invocations` endpoints.
//!
//! The agent-invocation envelope: `open` mints an accountability envelope (returns
//! its id), `close` terminates it with a disposition + opaque outcome, and
//! `show`/`list` read the envelope projections. Cogmap/invocation ids are substrate
//! UUIDs, not resource refs.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::invocation::{InvocationSummary, InvocationView};
use temper_core::types::invocation_requests::{
    CloseInvocationRequest, InvocationAck, OpenInvocationRequest,
};

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
        let path = "/api/invocations";
        let req_builder = self.http.post(path).json(req);
        self.http
            .send_json(&Method::POST, path, req_builder, Some(&token))
            .await
    }

    /// POST /api/invocations/{id}/close — terminate an open envelope. Returns
    /// **204 No Content**, so there is no body to deserialize.
    pub async fn close(&self, invocation_id: Uuid, req: &CloseInvocationRequest) -> Result<()> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/invocations/{invocation_id}/close");
        let req_builder = self.http.post(&path).json(req);
        self.http
            .send(&Method::POST, &path, req_builder, Some(&token))
            .await?;
        Ok(())
    }

    /// GET /api/invocations/{id} — read one envelope plus its acts.
    pub async fn show(&self, invocation_id: Uuid) -> Result<InvocationView> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/invocations/{invocation_id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
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
        let path = list_path(cogmap, status.as_deref());
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }
}

/// `/api/invocations` with optional `cogmap`/`status` query params — absent ones
/// are omitted. Shared by the method and its test.
fn list_path(cogmap: Option<Uuid>, status: Option<&str>) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(c) = cogmap {
        params.push(format!("cogmap={c}"));
    }
    if let Some(s) = status {
        params.push(format!("status={s}"));
    }
    if params.is_empty() {
        "/api/invocations".to_string()
    } else {
        format!("/api/invocations?{}", params.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_path_omits_all_when_none() {
        assert_eq!(list_path(None, None), "/api/invocations");
    }

    #[test]
    fn list_path_includes_cogmap_only() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            list_path(Some(id), None),
            format!("/api/invocations?cogmap={id}")
        );
    }

    #[test]
    fn list_path_includes_status_only() {
        assert_eq!(
            list_path(None, Some("open")),
            "/api/invocations?status=open"
        );
    }

    #[test]
    fn list_path_includes_both() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            list_path(Some(id), Some("completed")),
            format!("/api/invocations?cogmap={id}&status=completed")
        );
    }
}
