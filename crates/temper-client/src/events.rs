//! Typed sub-client for the `/api/events` endpoint.

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::api::{ContextChangesResponse, EventCursorResponse};
use temper_core::types::element_trail::{ElementKind, EventTrail};
use temper_core::types::query_params::ContextChangesQuery;
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

    /// GET /api/events/{kb_context_id}/cursor — the newest event id on a context among the events
    /// its change feed would serve the caller. `None` when the context is unreadable, has no such
    /// event, or none of its newest events qualifies; compare it for equality, never order.
    pub async fn latest_for_context(&self, kb_context_id: Uuid) -> Result<Option<Uuid>> {
        let token = self.http.resolve_token()?;
        let op = &ops::CURSOR;
        let path = op.path(&[&kb_context_id])?;
        let req = self.http.request(op, &path);
        let resp: EventCursorResponse = self
            .http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await?;
        Ok(resp.latest_event_id)
    }

    /// GET /api/events/{kb_context_id}/changes — a context's changes after a position, each a
    /// pointer (kind, subject, actor, time, act), never content. Omit `query.after` to start:
    /// the answer has no entries and a `through` at the present. A change hint, not a
    /// cache-invalidation guarantee: the feed never announces an admin act, so a mirror must
    /// periodically re-read the subjects it holds and drop what is refused.
    ///
    /// An unreadable context is [`crate::error::ClientError::NotFound`]; a refused position, bound or
    /// kind is a bad request.
    pub async fn changes(
        &self,
        kb_context_id: Uuid,
        query: &ContextChangesQuery,
    ) -> Result<ContextChangesResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::CONTEXT_CHANGES;
        let path = op.path(&[&kb_context_id])?;
        let req = self.http.request(op, &path).query(query);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
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
        let op = &ops::ELEMENT_TRAIL;
        let path = op.path(&[&kind_seg, &element_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }
}
