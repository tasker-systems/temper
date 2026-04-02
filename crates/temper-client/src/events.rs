//! Typed sub-client for the `/api/events` endpoint.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::{EventListParams, EventRow};

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

    /// List events, optionally filtered by resource or event type.
    pub async fn list(&self, params: &EventListParams) -> Result<Vec<EventRow>> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/events").query(params);
        self.http
            .send_json(&Method::GET, "/api/events", req, Some(&token))
            .await
    }
}
