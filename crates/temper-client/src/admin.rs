//! Typed sub-client for the `/api/access/admin/*` endpoints (Chunk 6).

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::access_gate::{
    CloseReviewBody, JoinRequest, JoinRequestStatus, JoinRequestWithProfile, QueueCount,
    ReconcileAutoJoinOutcome, ReviewRequestBody, ReviewRequestWithProfile, RevokePrincipalBody,
    SystemSettings,
};
use temper_core::types::admin::{
    AdminDirectoryListResponse, AdminLedgerQuery, AdminLedgerResponse, AdminProfileCard,
    AdminProfilesListQuery, DemoteAdminRequest, PromoteAdminRequest, ReembedRequest,
    ReembedSummary, UpdateSettingsRequest,
};
use temper_core::types::erasure::{
    BlockHistoryScrubExecuteResponse, BlockHistoryScrubRequestBody, BlockHistoryScrubSurvey,
    ErasureExecuteRequest, ErasureExecuteResponse, ErasureSurveyRequest, ErasureSurveyResponse,
    ResourceErasureExecuteRequest, ResourceErasureExecuteResponse, ResourceErasureSurvey,
    ResourceErasureSurveyRequest,
};
use temper_core::types::reblock::{ReblockReceipt, ReblockRequest};
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
        let op = &ops::ADMIN_GET_SETTINGS;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Partial-update system settings (admin only).
    pub async fn update_settings(&self, body: &UpdateSettingsRequest) -> Result<SystemSettings> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_UPDATE_SETTINGS;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Read the admin ledger. Exactly one axis — see [`AdminLedgerQuery`].
    ///
    /// Denies with **404, not 403**: a 403 would confirm the ledger has something to hide about
    /// the subject. So an error here means "nothing you may read", not "nothing exists".
    pub async fn ledger(&self, query: &AdminLedgerQuery) -> Result<AdminLedgerResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_ADMIN_LEDGER;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).query(query);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// The operator directory — the list page (admin only). See [`AdminProfilesListQuery`].
    ///
    /// Note this method never carries `?email=`: when that parameter is present the route answers
    /// a state card, not a page, and a method whose return type depended on its own arguments
    /// would put that dispatch in every caller. [`Self::profile_card_by_email`] is the card arm.
    pub async fn list_profiles(
        &self,
        query: &AdminProfilesListQuery,
    ) -> Result<AdminDirectoryListResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_LIST_PROFILES;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).query(query);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// The operator directory's identity-resolution arm — `?email=` on the list route (admin
    /// only). Resolves the address EXACTLY (case-insensitive, over verified auth-link emails)
    /// and answers the single matching profile's state card. Zero matches → `404`; more than one
    /// profile verified-owns the address → `404` whose body names the collision — the system
    /// refuses to pick (spec §6, review C1).
    ///
    /// This is the one place a human-controlled address becomes a target UUID, which is why it
    /// resolves server-side: every downstream admin act is strict-UUID, and a client-side
    /// substring resolution would let a lookalike address win (the wrong-principal enablement
    /// bridge).
    pub async fn profile_card_by_email(&self, email: &str) -> Result<AdminProfileCard> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_LIST_PROFILES;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).query(&AdminProfilesListQuery {
            email: Some(email.to_string()),
            ..Default::default()
        });
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// The principal state card (admin only) — the deep read behind the list row.
    pub async fn show_profile(&self, profile_id: Uuid) -> Result<AdminProfileCard> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_SHOW_PROFILE;
        let path = op.path(&[&profile_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Promote a profile to `owner` on a team (admin only).
    pub async fn promote(&self, body: &PromoteAdminRequest) -> Result<TeamMemberRow> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_PROMOTE;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Demote a system admin — revoke its governance grant (admin only). Returns `200 OK`, no body.
    ///
    /// The governance twin of [`Self::promote`]; the automatic path is demotion-by-transition
    /// (`revoke`/`deactivate` demote). Not team-scoped.
    pub async fn demote(&self, profile_id: Uuid) -> Result<()> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_DEMOTE;
        let path = op.path(&[])?;
        let req = self
            .http
            .request(op, &path)
            .json(&DemoteAdminRequest { profile_id });
        self.http
            .send(&op.method(), &path, req, Some(&token))
            .await?;
        Ok(())
    }

    /// List pending join requests for the gating team (admin only).
    pub async fn list_requests(&self) -> Result<Vec<JoinRequestWithProfile>> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_LIST_JOIN_REQUESTS;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// List undecided reconsideration requests — the D15 inbox (admin only).
    pub async fn list_reviews(&self) -> Result<Vec<ReviewRequestWithProfile>> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_LIST_REVIEWS;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/access/admin/requests/count — how many join requests are outstanding.
    ///
    /// [`Self::list_requests`] without the rows, for callers that report that a queue has
    /// something in it rather than answering it. A non-admin is refused with the same `403` the
    /// list raises, so "not yours to see" never arrives as a `0`.
    pub async fn count_requests(&self) -> Result<i32> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_COUNT_JOIN_REQUESTS;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        let body: QueueCount = self
            .http
            .send_json(&op.method(), &path, req, Some(&token))
            .await?;
        Ok(body.count)
    }

    /// GET /api/access/admin/reviews/count — how many reconsiderations are open.
    ///
    /// [`Self::list_reviews`] without the rows; same refusal posture as [`Self::count_requests`].
    pub async fn count_reviews(&self) -> Result<i32> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_COUNT_REVIEWS;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        let body: QueueCount = self
            .http
            .send_json(&op.method(), &path, req, Some(&token))
            .await?;
        Ok(body.count)
    }

    /// Record that a reconsideration was handled (admin only).
    ///
    /// Closing grants nothing — readmitting the principal is [`Self::approve_principal`]'s job, deliberately a
    /// separate call, so that a revocation can never be undone by the act of filing away the request
    /// to undo it.
    pub async fn close_review(
        &self,
        request_id: Uuid,
        decision_note: Option<String>,
    ) -> Result<()> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_CLOSE_REVIEW;
        let path = op.path(&[&request_id])?;
        let req = self
            .http
            .request(op, &path)
            .json(&CloseReviewBody { decision_note });
        self.http
            .send(&op.method(), &path, req, Some(&token))
            .await?;
        Ok(())
    }

    /// Trigger a re-embed for a scope of the index (admin only).
    ///
    /// Enqueues embed jobs for resources whose chunks were embedded with a model that is no longer the
    /// one the server embeds with; the per-minute drain does the actual work. Idempotent and safe to
    /// re-run — staleness is derived, not marked, so it only ever queues what genuinely needs it.
    pub async fn reembed(&self, body: &ReembedRequest) -> Result<ReembedSummary> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_REEMBED;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Run one bounded, resumable corpus re-blocking step (`POST /api/resources/reblock`).
    ///
    /// Re-blocks resources under the current chunking policy: `dry_run` surveys without
    /// touching anything (survey → act → re-survey), the scope names what the invocation
    /// covers (the deployment-wide `all` arm requires system-administrator standing), and
    /// the response is the receipt — per-candidate outcomes, per-class counts, the batch
    /// correlation id, and the continuation cursor. Idempotent per candidate: an
    /// already-conforming resource is a no-op that fires nothing.
    pub async fn reblock(&self, body: &ReblockRequest) -> Result<ReblockReceipt> {
        let token = self.http.resolve_token()?;
        let op = &ops::REBLOCK_RESOURCES;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
        let op = &ops::ADMIN_REVIEW_JOIN_REQUEST;
        let path = op.path(&[&request_id])?;
        let body = ReviewRequestBody {
            status: decision,
            decision_note,
        };
        let req = self.http.request(op, &path).json(&body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Approve a principal directly (admin only) — the machine/direct-grant door (D14/D16).
    pub async fn approve_principal(&self, profile_id: Uuid) -> Result<()> {
        self.standing_act(&ops::ADMIN_APPROVE_PRINCIPAL, profile_id, None)
            .await
    }

    /// Revoke a principal's admission (admin only). `reason` is required (D15).
    pub async fn revoke_principal(&self, profile_id: Uuid, reason: &str) -> Result<()> {
        self.standing_act(
            &ops::ADMIN_REVOKE_PRINCIPAL,
            profile_id,
            Some(RevokePrincipalBody {
                reason: reason.to_string(),
            }),
        )
        .await
    }

    /// Deactivate a principal (admin only).
    pub async fn deactivate_principal(&self, profile_id: Uuid) -> Result<()> {
        self.standing_act(&ops::ADMIN_DEACTIVATE_PRINCIPAL, profile_id, None)
            .await
    }

    /// Reactivate a deactivated principal, restoring its prior standing (admin only).
    pub async fn reactivate_principal(&self, profile_id: Uuid) -> Result<()> {
        self.standing_act(&ops::ADMIN_REACTIVATE_PRINCIPAL, profile_id, None)
            .await
    }

    /// Converge every auto-join team's roster to the standing-approved population (admin only).
    /// Returns the (team, profile) pairs added plus the touched teams that also carry SAML
    /// group mappings; an empty `added` means already converged.
    pub async fn reconcile_auto_join(&self) -> Result<ReconcileAutoJoinOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_RECONCILE_AUTO_JOIN;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(&serde_json::json!({}));
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Survey a principal's erasure (system admin only): what the act would redact and strike.
    /// Nothing is recorded or changed — a survey is not an erasure request.
    pub async fn survey_principal_erasure(
        &self,
        body: &ErasureSurveyRequest,
    ) -> Result<ErasureSurveyResponse> {
        self.erasure_door(&ops::ADMIN_SURVEY_PRINCIPAL_ERASURE, body)
            .await
    }

    /// Erase a principal (system admin only). `request_reference` is the operator's opaque DSAR
    /// reference. A repeat answers `already_erased`, not an error.
    ///
    /// Never retried: a write on this client's default send path is not replayed, and an erasure
    /// is the last thing that should be.
    pub async fn erase_principal(
        &self,
        body: &ErasureExecuteRequest,
    ) -> Result<ErasureExecuteResponse> {
        self.erasure_door(&ops::ADMIN_ERASE_PRINCIPAL, body).await
    }

    /// Survey a resource's erasure (system admin only). `plan` is absent when the resource was
    /// already erased. Nothing is recorded or changed.
    pub async fn survey_resource_erasure(
        &self,
        body: &ResourceErasureSurveyRequest,
    ) -> Result<ResourceErasureSurvey> {
        self.erasure_door(&ops::ADMIN_SURVEY_RESOURCE_ERASURE, body)
            .await
    }

    /// Erase a resource (system admin only). A refusal is an answer, not an error: the response's
    /// `status` says whether the act completed or was refused (and recorded). Never retried.
    pub async fn erase_resource(
        &self,
        body: &ResourceErasureExecuteRequest,
    ) -> Result<ResourceErasureExecuteResponse> {
        self.erasure_door(&ops::ADMIN_ERASE_RESOURCE, body).await
    }

    /// Survey a block-history scrub (system admin only): the `plan`, or the `refusal` the act
    /// would record. Nothing is recorded or changed.
    pub async fn survey_block_history_scrub(
        &self,
        body: &BlockHistoryScrubRequestBody,
    ) -> Result<BlockHistoryScrubSurvey> {
        self.erasure_door(&ops::ADMIN_SURVEY_BLOCK_HISTORY_SCRUB, body)
            .await
    }

    /// Scrub the named blocks' revision history (system admin only). A refusal is an answer, not
    /// an error: the response's `status` says which. Never retried.
    pub async fn scrub_block_history(
        &self,
        body: &BlockHistoryScrubRequestBody,
    ) -> Result<BlockHistoryScrubExecuteResponse> {
        self.erasure_door(&ops::ADMIN_SCRUB_BLOCK_HISTORY, body)
            .await
    }

    /// Shared POST for the erasure family's six doors: a JSON body in, a JSON answer out.
    async fn erasure_door<B, T>(&self, op: &ops::Op, body: &B) -> Result<T>
    where
        B: serde::Serialize + ?Sized,
        T: serde::de::DeserializeOwned,
    {
        let token = self.http.resolve_token()?;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Shared POST for the standing acts. They return `200 OK` with no body.
    async fn standing_act(
        &self,
        op: &ops::Op,
        profile_id: Uuid,
        body: Option<RevokePrincipalBody>,
    ) -> Result<()> {
        let token = self.http.resolve_token()?;
        let path = op.path(&[&profile_id])?;
        let mut req = self.http.request(op, &path);
        if let Some(body) = body {
            req = req.json(&body);
        }
        self.http
            .send(&op.method(), &path, req, Some(&token))
            .await?;
        Ok(())
    }
}
