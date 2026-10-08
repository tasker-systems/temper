//! Typed sub-client for the `/api/steward` endpoints (T4a).
//!
//! `delta` reads a team-self-cognition cogmap's ingest delta since its watermark; `advance_watermark`
//! moves the cursor forward. The cogmap is a substrate UUID (the CLI resolves any decorated ref to
//! its trailing UUID before calling).

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;

/// The header the steward dispatch door reads its per-tick correlation id from.
const STEWARD_CORRELATION_HEADER: &str = "x-steward-correlation-id";
use temper_core::types::query_params::DeltaQuery;
use temper_core::types::steward::{
    AdvanceWatermarkAck, AdvanceWatermarkRequest, DispatchTickRequest, DispatchTickResponse,
    DriftSweepRow, IngestDelta,
};

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
        let op = &ops::DELTA;
        let path = delta_path(cogmap, threshold)?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/steward/{cogmap}/watermark — record a completed run's cursors. Both are optional:
    /// no `event_id` is a boundary-only advance (the watermark holds), and no `boundary_fingerprint`
    /// makes the server settle the boundary to its write-time shape. The ack reports what was
    /// actually stored, which is not necessarily what was sent.
    pub async fn advance_watermark(
        &self,
        cogmap: Uuid,
        event_id: Option<Uuid>,
        boundary_fingerprint: Option<String>,
    ) -> Result<AdvanceWatermarkAck> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADVANCE;
        let path = op.path(&[&cogmap])?;
        let body = AdvanceWatermarkRequest {
            event_id,
            boundary_fingerprint,
        };
        let req = self.http.request(op, &path).json(&body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/steward/sweep — steward drift across the maps the caller can steward, measured
    /// against `query.threshold` (the server default when absent).
    pub async fn sweep(&self, query: &DeltaQuery) -> Result<Vec<DriftSweepRow>> {
        let token = self.http.resolve_token()?;
        let op = &ops::STEWARD_SWEEP;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).query(query);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/steward/candidates — the cognitive maps the caller may steward.
    pub async fn candidates(&self) -> Result<Vec<Uuid>> {
        let token = self.http.resolve_token()?;
        let op = &ops::CANDIDATES;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/steward/dispatch — claim the drifted maps for one dispatch tick. `correlation_id`
    /// rides `x-steward-correlation-id` and is stamped onto every claimed job; the response echoes
    /// what the server actually stamped (`None` when it was absent).
    pub async fn dispatch(
        &self,
        request: &DispatchTickRequest,
        correlation_id: Option<Uuid>,
    ) -> Result<DispatchTickResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::STEWARD_DISPATCH;
        let path = op.path(&[])?;
        let mut req = self.http.request(op, &path).json(request);
        if let Some(id) = correlation_id {
            req = req.header(STEWARD_CORRELATION_HEADER, id.to_string());
        }
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}

/// `/api/steward/{cogmap}/delta` with an optional `threshold` query param — omitted when absent.
/// Shared by the method and its test.
fn delta_path(cogmap: Uuid, threshold: Option<i64>) -> Result<String> {
    let base = ops::DELTA.path(&[&cogmap])?;
    Ok(match threshold {
        Some(t) => format!("{base}?threshold={t}"),
        None => base,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_path_omits_threshold_when_none() {
        let id = Uuid::from_u128(7);
        assert_eq!(delta_path(id, None).unwrap(), format!("/api/steward/{id}/delta"));
    }

    #[test]
    fn delta_path_includes_threshold() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            delta_path(id, Some(5)).unwrap(),
            format!("/api/steward/{id}/delta?threshold=5")
        );
    }
}
