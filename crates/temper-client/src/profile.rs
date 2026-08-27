//! Typed sub-client for the `/api/profile` endpoints.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::ProfileUpdateRequest;
use temper_core::types::profile::{Profile, ProfileAuthLink, ProfileWithEntitlements};

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
    ///
    /// Deserializes into a bare [`Profile`], which **drops the `entitlements` object the endpoint
    /// also returns** — `Profile` neither declares that field nor denies unknown ones, so serde
    /// discards it without complaint. That is fine for the callers who want identity only; anything
    /// asking about access must use [`get_with_entitlements`](Self::get_with_entitlements), or it
    /// will make the round trip and throw the answer away.
    pub async fn get(&self) -> Result<Profile> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/profile");
        self.http
            .send_json(&Method::GET, "/api/profile", req, Some(&token))
            .await
    }

    /// Get the authenticated user's profile **with** their system-level entitlements.
    ///
    /// Same endpoint and same round trip as [`get`](Self::get) — the entitlements have always been
    /// in the response body; only the type read back is different. This is the authoritative answer
    /// to "may I use this instance?": `entitlements.system_access` is the SQL `has_system_access`
    /// predicate, which reads `kb_principal_standing`. Do not reconstruct that answer from the join
    /// request queue — an approved principal who never filed a request has no row there, so the
    /// queue reports denial for exactly the people most likely to hold access.
    pub async fn get_with_entitlements(&self) -> Result<ProfileWithEntitlements> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/profile");
        self.http
            .send_json(&Method::GET, "/api/profile", req, Some(&token))
            .await
    }

    /// Update the authenticated user's profile.
    pub async fn update(&self, request: &ProfileUpdateRequest) -> Result<Profile> {
        let token = self.http.resolve_token()?;
        let req = self.http.patch("/api/profile").json(request);
        self.http
            .send_json(&Method::PATCH, "/api/profile", req, Some(&token))
            .await
    }

    /// List external auth provider links for the authenticated user.
    pub async fn auth_links(&self) -> Result<Vec<ProfileAuthLink>> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/profile/auth-links");
        self.http
            .send_json(&Method::GET, "/api/profile/auth-links", req, Some(&token))
            .await
    }
}
