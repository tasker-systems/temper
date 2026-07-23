//! Typed sub-client for the `/api/access` endpoints.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::access_gate::{JoinRequest, PublicSystemSettings};

/// Request body for creating a join request.
#[derive(serde::Serialize)]
struct CreateRequestBody<'a> {
    message: Option<&'a str>,
    source: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    accepted_terms_version: Option<&'a str>,
}

/// Request body for a review request (D15 — a revoked principal asking for reconsideration).
#[derive(serde::Serialize)]
struct CreateReviewBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
}

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
            message,
            source,
            accepted_terms_version,
        };
        let req = self.http.post("/api/access/requests").json(&body);
        self.http
            .send_json(&Method::POST, "/api/access/requests", req, Some(&token))
            .await
    }

    /// Get the caller's most recent join request (if any).
    pub async fn get_own_request(&self) -> Result<Option<JoinRequest>> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/access/requests/me");
        self.http
            .send_json(&Method::GET, "/api/access/requests/me", req, Some(&token))
            .await
    }

    /// Withdraw a pending join request.
    pub async fn withdraw_request(&self) -> Result<()> {
        let token = self.http.resolve_token()?;
        let req = self.http.delete("/api/access/requests/me");
        self.http
            .send(
                &Method::DELETE,
                "/api/access/requests/me",
                req,
                Some(&token),
            )
            .await?;
        Ok(())
    }

    /// Ask an admin to reconsider a revocation (D15). Does not restore access by itself.
    pub async fn create_review_request(&self, message: Option<&str>) -> Result<()> {
        let token = self.http.resolve_token()?;
        let body = CreateReviewBody { message };
        let req = self.http.post("/api/access/reviews").json(&body);
        self.http
            .send(&Method::POST, "/api/access/reviews", req, Some(&token))
            .await?;
        Ok(())
    }

    /// Get the public system settings (access mode, terms info).
    pub async fn get_settings(&self) -> Result<PublicSystemSettings> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/access/settings");
        self.http
            .send_json(&Method::GET, "/api/access/settings", req, Some(&token))
            .await
    }
}
