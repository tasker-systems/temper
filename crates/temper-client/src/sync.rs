//! Typed sub-client for the `/api/sync` endpoints.
//!
//! Provides typed methods for computing sync diffs and completing sync rounds.

use crate::auth;
use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::{
    SyncCompleteRequest, SyncCompleteResponse, SyncStatusRequest, SyncStatusResponse,
};

/// Sub-client for sync operations.
pub struct SyncClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for SyncClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncClient").finish_non_exhaustive()
    }
}

impl<'a> SyncClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/sync/status — compute diff between local manifest and server state.
    pub async fn status(&self, request: &SyncStatusRequest) -> Result<SyncStatusResponse> {
        let token = auth::current_token()?;
        let req = self.http.post("/api/sync/status").json(request);
        self.http.send_json(req, Some(&token)).await
    }

    /// POST /api/sync/complete — finalize a sync round, update device state.
    pub async fn complete(&self, request: &SyncCompleteRequest) -> Result<SyncCompleteResponse> {
        let token = auth::current_token()?;
        let req = self.http.post("/api/sync/complete").json(request);
        self.http.send_json(req, Some(&token)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpClient;

    #[test]
    fn sync_client_is_debug() {
        let client = HttpClient::new("https://example.com", None);
        let sync = SyncClient::new(&client);
        let debug_str = format!("{sync:?}");
        assert!(debug_str.contains("SyncClient"));
    }
}
