//! Typed sub-client for the `/api/profile` endpoints.

use reqwest::Method;

use crate::auth;
use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::ProfileUpdateRequest;
use temper_core::types::profile::{Profile, ProfileAuthLink};

/// Sub-client for profile operations.
pub struct ProfileClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for ProfileClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileClient").finish_non_exhaustive()
    }
}

impl<'a> ProfileClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Get the authenticated user's profile.
    pub async fn get(&self) -> Result<Profile> {
        let token = auth::current_token()?;
        let req = self.http.get("/api/profile");
        self.http
            .send_json(&Method::GET, "/api/profile", req, Some(&token))
            .await
    }

    /// Update the authenticated user's profile.
    pub async fn update(&self, request: &ProfileUpdateRequest) -> Result<Profile> {
        let token = auth::current_token()?;
        let req = self.http.patch("/api/profile").json(request);
        self.http
            .send_json(&Method::PATCH, "/api/profile", req, Some(&token))
            .await
    }

    /// List external auth provider links for the authenticated user.
    pub async fn auth_links(&self) -> Result<Vec<ProfileAuthLink>> {
        let token = auth::current_token()?;
        let req = self.http.get("/api/profile/auth-links");
        self.http
            .send_json(&Method::GET, "/api/profile/auth-links", req, Some(&token))
            .await
    }
}
