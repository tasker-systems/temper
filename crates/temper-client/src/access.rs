//! Typed sub-client for the `/api/access` endpoints.

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::access_gate::{
    CreateRequestBody, CreateReviewBody, JoinRequest, PublicSystemSettings,
};

/// Sub-client for system access operations.
pub struct AccessClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for AccessClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessClient").finish_non_exhaustive()
    }
}

impl<'a> AccessClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Submit a join request for the system gating team.
    pub async fn create_request(
        &self,
        message: Option<&str>,
        source: &str,
        accepted_terms_version: Option<&str>,
    ) -> Result<JoinRequest> {
        let token = self.http.resolve_token()?;
        let body = CreateRequestBody {
            message: message.map(str::to_string),
            source: source.to_string(),
            accepted_terms_version: accepted_terms_version.map(str::to_string),
        };
        let op = &ops::CREATE_REQUEST;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(&body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Get the caller's most recent join request (if any).
    pub async fn get_own_request(&self) -> Result<Option<JoinRequest>> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_OWN_REQUEST;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Withdraw a pending join request.
    pub async fn withdraw_request(&self) -> Result<()> {
        let token = self.http.resolve_token()?;
        let op = &ops::WITHDRAW_REQUEST;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send(&op.method(), &path, req, Some(&token))
            .await?;
        Ok(())
    }

    /// Ask an admin to reconsider a revocation (D15). Does not restore access by itself.
    pub async fn create_review_request(&self, message: Option<&str>) -> Result<()> {
        let token = self.http.resolve_token()?;
        let body = CreateReviewBody {
            message: message.map(str::to_string),
        };
        let op = &ops::CREATE_REVIEW_REQUEST;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(&body);
        self.http
            .send(&op.method(), &path, req, Some(&token))
            .await?;
        Ok(())
    }

    /// Get the public system settings (access mode, terms info).
    pub async fn get_settings(&self) -> Result<PublicSystemSettings> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_SETTINGS;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}
