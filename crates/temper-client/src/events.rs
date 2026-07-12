//! Typed sub-client for the `/api/events` endpoint.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::EventCursorResponse;
use temper_core::types::element_trail::{ElementKind, EventTrail};
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

    /// GET /api/graph/elements/{kind}/{id}/trail — the time-ordered event trail
    /// (append-only history) of a single graph element: a node (resource) or an
    /// edge. Visibility is gated server-side; an unreadable or nonexistent element
    /// yields an empty trail rather than an error.
    pub async fn element_trail(&self, kind: ElementKind, element_id: Uuid) -> Result<EventTrail> {
        let token = self.http.resolve_token()?;
        // The route segment is the lowercase kind name; map it explicitly rather than
        // leaning on the serde rename so the path form is greppable at the call site.
        let kind_seg = match kind {
            ElementKind::Node => "node",
            ElementKind::Edge => "edge",
        };
        let path = format!("/api/graph/elements/{kind_seg}/{element_id}/trail");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }
}
