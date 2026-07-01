//! Typed sub-client for the `/api/steward` endpoints (T4a).
//!
//! `delta` reads a team-self-cognition cogmap's ingest delta since its watermark; `advance_watermark`
//! moves the cursor forward. The cogmap is a substrate UUID (the CLI resolves any decorated ref to
//! its trailing UUID before calling).

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::steward::{AdvanceWatermarkAck, AdvanceWatermarkRequest, IngestDelta};

/// Sub-client for steward ingest-trigger operations.
pub struct StewardClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for StewardClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StewardClient").finish_non_exhaustive()
    }
}

impl<'a> StewardClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// GET /api/steward/{cogmap}/delta[?threshold=] — read the ingest delta.
    pub async fn delta(&self, cogmap: Uuid, threshold: Option<i64>) -> Result<IngestDelta> {
        let token = self.http.resolve_token()?;
        let path = delta_path(cogmap, threshold);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// POST /api/steward/{cogmap}/watermark — advance the ingest watermark to `event_id`.
    pub async fn advance_watermark(
        &self,
        cogmap: Uuid,
        event_id: Uuid,
    ) -> Result<AdvanceWatermarkAck> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/steward/{cogmap}/watermark");
        let body = AdvanceWatermarkRequest { event_id };
        let req = self.http.post(&path).json(&body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }
}

/// `/api/steward/{cogmap}/delta` with an optional `threshold` query param — omitted when absent.
/// Shared by the method and its test.
fn delta_path(cogmap: Uuid, threshold: Option<i64>) -> String {
    match threshold {
        Some(t) => format!("/api/steward/{cogmap}/delta?threshold={t}"),
        None => format!("/api/steward/{cogmap}/delta"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_path_omits_threshold_when_none() {
        let id = Uuid::from_u128(7);
        assert_eq!(delta_path(id, None), format!("/api/steward/{id}/delta"));
    }

    #[test]
    fn delta_path_includes_threshold() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            delta_path(id, Some(5)),
            format!("/api/steward/{id}/delta?threshold=5")
        );
    }
}
