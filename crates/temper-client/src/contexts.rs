//! Typed sub-client for the `/api/contexts` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::context_ref::ContextOwnerRef;
use temper_core::types::context::{
    ContextCreateRequest, ContextRow, ContextRowWithCounts, ShareContextOutcome,
    ShareContextRequest, UnshareContextOutcome,
};

/// Sub-client for context operations.
pub struct ContextClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for ContextClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextClient").finish_non_exhaustive()
    }
}

impl<'a> ContextClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// List all visible contexts with resource counts.
    pub async fn list(&self) -> Result<Vec<ContextRowWithCounts>> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/contexts");
        self.http
            .send_json(&Method::GET, "/api/contexts", req, Some(&token))
            .await
    }

    /// Get a single context by ID.
    pub async fn get(&self, id: Uuid) -> Result<ContextRow> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Create a new context. `owner` is `None` for a profile-owned context (the
    /// default) or `Some(ContextOwnerRef::Team(slug))` for a team-owned one
    /// (role-gated server-side).
    pub async fn create(&self, name: &str, owner: Option<ContextOwnerRef>) -> Result<ContextRow> {
        let token = self.http.resolve_token()?;
        let body = ContextCreateRequest {
            name: name.to_owned(),
            owner,
        };
        let req = self.http.post("/api/contexts").json(&body);
        self.http
            .send_json(&Method::POST, "/api/contexts", req, Some(&token))
            .await
    }

    /// POST /api/contexts/{id}/teams — share the context into a team (admin-gated, idempotent).
    pub async fn share_team(
        &self,
        context_id: Uuid,
        body: &ShareContextRequest,
    ) -> Result<ShareContextOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{context_id}/teams");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// DELETE /api/contexts/{id}/teams/{team_id} — unshare (admin-gated, no-op safe).
    pub async fn unshare_team(
        &self,
        context_id: Uuid,
        team_id: Uuid,
    ) -> Result<UnshareContextOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{context_id}/teams/{team_id}");
        let req = self.http.delete(&path);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }
}
