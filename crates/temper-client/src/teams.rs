//! Typed sub-client for the `/api/teams` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamMemberRow, TeamRow};

/// Sub-client for team lifecycle operations (create / add-member / list).
pub struct TeamsClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for TeamsClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeamsClient").finish_non_exhaustive()
    }
}

impl<'a> TeamsClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// List the teams the caller is a member of.
    pub async fn list(&self) -> Result<Vec<TeamRow>> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/teams");
        self.http
            .send_json(&Method::GET, "/api/teams", req, Some(&token))
            .await
    }

    /// Create a team (the caller becomes its `owner`).
    pub async fn create(&self, body: &TeamCreateRequest) -> Result<TeamRow> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/teams").json(body);
        self.http
            .send_json(&Method::POST, "/api/teams", req, Some(&token))
            .await
    }

    /// Add (or update) a member on a team.
    pub async fn add_member(
        &self,
        team_id: Uuid,
        body: &AddMemberRequest,
    ) -> Result<TeamMemberRow> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}/members");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }
}
