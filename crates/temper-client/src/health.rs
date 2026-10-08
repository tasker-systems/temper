//! Typed sub-client for `GET /api/health` — the one door that asks for no credential.

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::api::HealthResponse;

/// Sub-client for the service health read.
pub struct HealthClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for HealthClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HealthClient").finish_non_exhaustive()
    }
}

impl<'a> HealthClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// GET /api/health — the service's status, version, and the commit it was built from (`None`
    /// when the build recorded none). Sends no token: the door is unauthenticated, and a health
    /// probe must not fail because the caller is logged out.
    pub async fn get_health(&self) -> Result<HealthResponse> {
        let op = &ops::HEALTH_CHECK;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http.send_json(&op.method(), &path, req, None).await
    }
}
