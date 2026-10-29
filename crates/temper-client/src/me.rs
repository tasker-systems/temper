//! Typed sub-client for the `/api/me` endpoints: reads about the caller's own work.

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::context::ContextActivity;

/// Sub-client for reads about the caller's own work.
pub struct MeClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for MeClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeClient").finish_non_exhaustive()
    }
}

impl<'a> MeClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// GET `/api/me/context-activity` — when the caller last changed something in each context
    /// they can read: one row per context they ever changed something in, newest first. Their own
    /// authored changes only; a context with no row is one they cannot read or never changed.
    pub async fn context_activity(&self) -> Result<Vec<ContextActivity>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_MY_CONTEXT_ACTIVITY;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }
}
