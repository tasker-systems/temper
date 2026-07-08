//! Typed sub-client for the `/api/teams` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::invitation::{
    AcceptInvitationResponse, CreateInvitationRequest, InviteeInvitation, TeamInvitation,
};
use temper_core::types::reassign::{BulkReassignAck, BulkReassignRequest};
use temper_core::types::team::{
    AddMemberRequest, ChangeRoleRequest, TeamCreateRequest, TeamDetail, TeamMemberRow, TeamRow,
    TeamUpdateRequest,
};

/// Sub-client for team lifecycle operations (create / add-member / list /
/// detail / remove-member / change-role).
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

    /// GET /api/teams/{id} — team detail + member roster.
    pub async fn get(&self, team_id: Uuid) -> Result<TeamDetail> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// PATCH /api/teams/{id} — update team metadata (name/description).
    pub async fn update(&self, team_id: Uuid, body: &TeamUpdateRequest) -> Result<TeamRow> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}");
        let req = self.http.patch(&path).json(body);
        self.http
            .send_json(&Method::PATCH, &path, req, Some(&token))
            .await
    }

    /// DELETE /api/teams/{id} — soft-delete a team (owner only).
    ///
    /// Returns `()` on a 204; `send` errors on any non-2xx (403/404/409), so
    /// callers surface the guard failures without decoding a body.
    pub async fn delete(&self, team_id: Uuid) -> Result<()> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}");
        let req = self.http.delete(&path);
        self.http
            .send(&Method::DELETE, &path, req, Some(&token))
            .await?;
        Ok(())
    }

    /// PATCH /api/teams/{id}/members/{profile_id} — change a member's role.
    pub async fn change_role(
        &self,
        team_id: Uuid,
        profile_id: Uuid,
        body: &ChangeRoleRequest,
    ) -> Result<TeamMemberRow> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}/members/{profile_id}");
        let req = self.http.patch(&path).json(body);
        self.http
            .send_json(&Method::PATCH, &path, req, Some(&token))
            .await
    }

    /// DELETE /api/teams/{id}/members/{profile_id} — remove a member (or self-leave).
    ///
    /// Returns `()` on a 204; `send` errors on any non-2xx (403/404/409), so
    /// callers surface the guard failures without decoding a body.
    pub async fn remove_member(&self, team_id: Uuid, profile_id: Uuid) -> Result<()> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}/members/{profile_id}");
        let req = self.http.delete(&path);
        self.http
            .send(&Method::DELETE, &path, req, Some(&token))
            .await?;
        Ok(())
    }

    /// POST /api/teams/{id}/invite — create a pending invitation.
    pub async fn invite(
        &self,
        team_id: Uuid,
        body: &CreateInvitationRequest,
    ) -> Result<TeamInvitation> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}/invite");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// POST /api/teams/{id}/reassign — bulk-reassign the team's resources.
    pub async fn reassign(
        &self,
        team_id: Uuid,
        body: &BulkReassignRequest,
    ) -> Result<BulkReassignAck> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}/reassign");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// GET /api/teams/{id}/invitations — list pending invitations.
    pub async fn list_invitations(&self, team_id: Uuid) -> Result<Vec<TeamInvitation>> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}/invitations");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// GET /api/invitations/mine — the caller's own pending invitations.
    pub async fn list_my_invitations(&self) -> Result<Vec<InviteeInvitation>> {
        let token = self.http.resolve_token()?;
        let path = "/api/invitations/mine";
        let req = self.http.get(path);
        self.http
            .send_json(&Method::GET, path, req, Some(&token))
            .await
    }

    /// POST /api/invitations/{token}/accept — redeem an invitation token.
    pub async fn accept_invitation(&self, invite_token: &str) -> Result<AcceptInvitationResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/invitations/{invite_token}/accept");
        let req = self.http.post(&path);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// POST /api/invitations/{token}/decline — decline an invitation token.
    ///
    /// Returns `()` on a 204; `send` errors on any non-2xx.
    pub async fn decline_invitation(&self, invite_token: &str) -> Result<()> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/invitations/{invite_token}/decline");
        let req = self.http.post(&path);
        self.http
            .send(&Method::POST, &path, req, Some(&token))
            .await?;
        Ok(())
    }
}
