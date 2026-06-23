//! Typed sub-client for the `/api/events` endpoint.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::EventCursorResponse;
use uuid::Uuid;

/// Sub-client for event listing.
pub struct EventClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for EventClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventClient").finish_non_exhaustive()
    }
}

impl<'a> EventClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// GET /api/events/{kb_context_id}/cursor — the most recent event id for
    /// a context.
    pub async fn latest_for_context(&self, kb_context_id: Uuid) -> Result<Option<Uuid>> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/events/{kb_context_id}/cursor");
        let req = self.http.get(&path);
        let resp: EventCursorResponse = self
            .http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await?;
        Ok(resp.latest_event_id)
    }
}
