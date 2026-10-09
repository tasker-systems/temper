//! Typed sub-client for the `/api/auditor` worker doors.
//!
//! `dispatch` claims a batch of citation-audit jobs for one tick; `complete` closes the caller's
//! in-flight job on a cogmap. The cogmap is a substrate UUID.

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::auditor::{
    AuditSweepRow, AuditorDispatchTickRequest, AuditorDispatchTickResponse, AuditorJobCompleteAck,
};
use temper_core::types::query_params::SweepQuery;

/// The header the auditor dispatch door reads its per-tick correlation id from — its own name, not
/// the steward's.
const AUDITOR_CORRELATION_HEADER: &str = "x-auditor-correlation-id";

/// Sub-client for auditor worker operations.
pub struct AuditorClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for AuditorClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditorClient").finish_non_exhaustive()
    }
}

impl<'a> AuditorClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/auditor/dispatch — claim a batch of auditor jobs. `correlation_id` rides
    /// `x-auditor-correlation-id` and is stamped onto every claimed job; the response echoes what
    /// the server actually stamped (`None` when it was absent).
    pub async fn dispatch(
        &self,
        request: &AuditorDispatchTickRequest,
        correlation_id: Option<Uuid>,
    ) -> Result<AuditorDispatchTickResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::AUDITOR_DISPATCH;
        let path = op.path(&[])?;
        let mut req = self.http.request(op, &path).json(request);
        if let Some(id) = correlation_id {
            req = req.header(AUDITOR_CORRELATION_HEADER, id.to_string());
        }
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/auditor/sweep — audit coverage across the findings the caller can read, up to
    /// `query.cap` of them.
    pub async fn sweep(&self, query: &SweepQuery) -> Result<Vec<AuditSweepRow>> {
        let token = self.http.resolve_token()?;
        let op = &ops::AUDITOR_SWEEP;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).query(query);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/auditor/{cogmap}/complete — complete the caller's in-flight job on this cogmap.
    /// There is no body: the outcome lives in the audit trail the session wrote. `job_id` is `None`
    /// when nothing of the caller's was in flight.
    pub async fn complete(&self, cogmap: Uuid) -> Result<AuditorJobCompleteAck> {
        let token = self.http.resolve_token()?;
        let op = &ops::COMPLETE_AUDITOR_JOB;
        let path = op.path(&[&cogmap])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}
