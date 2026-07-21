//! Typed sub-client for the `/api/access/admin/*` endpoints (Chunk 6).

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::access_gate::{
    JoinRequest, JoinRequestStatus, JoinRequestWithProfile, SystemSettings,
};
use temper_core::types::admin::{
    AdminLedgerQuery, AdminLedgerResponse, PromoteAdminRequest, ReembedRequest, ReembedSummary,
    UpdateSettingsRequest,
};
use temper_core::types::team::TeamMemberRow;

/// Sub-client for admin / system-settings operations.
pub struct AdminClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for AdminClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminClient").finish_non_exhaustive()
    }
}

impl<'a> AdminClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Read full system settings (admin only).
    pub async fn get_settings(&self) -> Result<SystemSettings> {
        let token = self.http.resolve_token()?;
        let path = "/api/access/admin/settings";
        let req = self.http.get(path);
        self.http
            .send_json(&Method::GET, path, req, Some(&token))
            .await
    }

    /// Partial-update system settings (admin only).
    pub async fn update_settings(&self, body: &UpdateSettingsRequest) -> Result<SystemSettings> {
        let token = self.http.resolve_token()?;
        let path = "/api/access/admin/settings";
        let req = self.http.patch(path).json(body);
        self.http
            .send_json(&Method::PATCH, path, req, Some(&token))
            .await
    }

    /// Read the admin ledger. Exactly one axis — see [`AdminLedgerQuery`].
    ///
    /// Denies with **404, not 403**: a 403 would confirm the ledger has something to hide about
    /// the subject. So an error here means "nothing you may read", not "nothing exists".
    pub async fn ledger(&self, query: &AdminLedgerQuery) -> Result<AdminLedgerResponse> {
        let token = self.http.resolve_token()?;
        let path = "/api/admin/ledger";
        let req = self.http.get(path).query(query);
        self.http
            .send_json(&Method::GET, path, req, Some(&token))
            .await
    }

    /// Promote a profile to `owner` on a team (admin only).
    pub async fn promote(&self, body: &PromoteAdminRequest) -> Result<TeamMemberRow> {
        let token = self.http.resolve_token()?;
        let path = "/api/access/admin/promote";
        let req = self.http.post(path).json(body);
        self.http
            .send_json(&Method::POST, path, req, Some(&token))
            .await
    }

    /// List pending join requests for the gating team (admin only).
    pub async fn list_requests(&self) -> Result<Vec<JoinRequestWithProfile>> {
        let token = self.http.resolve_token()?;
        let path = "/api/access/admin/requests";
        let req = self.http.get(path);
        self.http
            .send_json(&Method::GET, path, req, Some(&token))
            .await
    }

    /// Trigger a re-embed for a scope of the index (admin only).
    ///
    /// Enqueues embed jobs for resources whose chunks were embedded with a model that is no longer the
    /// one the server embeds with; the per-minute drain does the actual work. Idempotent and safe to
    /// re-run — staleness is derived, not marked, so it only ever queues what genuinely needs it.
    pub async fn reembed(&self, body: &ReembedRequest) -> Result<ReembedSummary> {
        let token = self.http.resolve_token()?;
        let path = "/api/embed/admin/reembed";
        let req = self.http.post(path).json(body);
        self.http
            .send_json(&Method::POST, path, req, Some(&token))
            .await
    }

    /// Approve or reject a join request (admin only).
    pub async fn review_request(
        &self,
        request_id: Uuid,
        decision: JoinRequestStatus,
        decision_note: Option<String>,
    ) -> Result<JoinRequest> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/access/admin/requests/{request_id}");
        let body = ReviewBody {
            status: decision,
            decision_note,
        };
        let req = self.http.patch(&path).json(&body);
        self.http
            .send_json(&Method::PATCH, &path, req, Some(&token))
            .await
    }

    /// Approve a principal directly (admin only) — the machine/direct-grant door (D14/D16).
    pub async fn approve_principal(&self, profile_id: Uuid) -> Result<()> {
        self.standing_act(profile_id, "approve", None).await
    }

    /// Revoke a principal's admission (admin only). `reason` is required (D15).
    pub async fn revoke_principal(&self, profile_id: Uuid, reason: &str) -> Result<()> {
        self.standing_act(profile_id, "revoke", Some(RevokeBody { reason }))
            .await
    }

    /// Deactivate a principal (admin only).
    pub async fn deactivate_principal(&self, profile_id: Uuid) -> Result<()> {
        self.standing_act(profile_id, "deactivate", None).await
    }

    /// Reactivate a deactivated principal, restoring its prior standing (admin only).
    pub async fn reactivate_principal(&self, profile_id: Uuid) -> Result<()> {
        self.standing_act(profile_id, "reactivate", None).await
    }

    /// Shared POST for the standing acts. They return `200 OK` with no body.
    async fn standing_act(
        &self,
        profile_id: Uuid,
        verb: &str,
        body: Option<RevokeBody<'_>>,
    ) -> Result<()> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/access/admin/principals/{profile_id}/{verb}");
        let mut req = self.http.post(&path);
        if let Some(body) = body {
            req = req.json(&body);
        }
        self.http
            .send(&Method::POST, &path, req, Some(&token))
            .await?;
        Ok(())
    }
}

/// Mirrors `handlers::access::RevokePrincipalBody`.
#[derive(serde::Serialize)]
struct RevokeBody<'a> {
    reason: &'a str,
}

/// Mirrors `handlers::access::ReviewRequestBody` (the handler's private body type).
#[derive(serde::Serialize)]
struct ReviewBody {
    status: JoinRequestStatus,
    decision_note: Option<String>,
}
