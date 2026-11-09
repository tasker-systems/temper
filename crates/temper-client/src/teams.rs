//! Typed sub-client for the `/api/teams` endpoints.

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::invitation::{
    AcceptInvitationResponse, CreateInvitationRequest, InvitationTokenRequest, InviteeInvitation,
    PendingInvitationCounts, TeamInvitation,
};
use temper_core::types::reassign::{
    BulkReassignAck, BulkReassignRequest, RemoveMemberOutcome, ResidualOwnedReach,
};
use temper_core::types::team::{
    AddMemberRequest, AttachChildOutcome, ChangeRoleRequest, PublicWithinEffects,
    PublicWithinOutcome, PublicWithinPolicyOutcome, PublicWithinPolicyRequest, PublicWithinRowRef,
    RenameTeamSlugOutcome, RenameTeamSlugRequest, ReparentOutcome, ReparentRequest,
    RestoreTeamOutcome, SetAutoJoinOutcome, SetAutoJoinRequest, TeamActAnswerQuery,
    TeamCreateRequest, TeamDetail, TeamMemberRow, TeamRow, TeamUpdateRequest,
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
        let op = &ops::LIST_TEAMS;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// Create a team (the caller becomes its `owner`). Asks for the act's `public_within` effects
    /// (`?answer=effects`): a server that knows the parameter fills `TeamRow.public_within` and
    /// `public_within_in_reach`; an older one ignores it and answers the row alone, both `None`.
    pub async fn create(&self, body: &TeamCreateRequest) -> Result<TeamRow> {
        let token = self.http.resolve_token()?;
        let op = &ops::CREATE_TEAM;
        let path = op.path(&[])?;
        let req = self
            .http
            .request(op, &path)
            .query(&TeamActAnswerQuery::effects())
            .json(body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// Add a member to a team. A profile already on the team is a 409; change its role with
    /// [`Self::change_role`].
    pub async fn add_member(
        &self,
        team_id: Uuid,
        body: &AddMemberRequest,
    ) -> Result<TeamMemberRow> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADD_MEMBER;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// GET /api/teams/{id} — team detail + member roster.
    pub async fn get(&self, team_id: Uuid) -> Result<TeamDetail> {
        let token = self.http.resolve_token()?;
        let op = &ops::DETAIL;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// PATCH /api/teams/{id} — update team metadata (name/description).
    pub async fn update(&self, team_id: Uuid, body: &TeamUpdateRequest) -> Result<TeamRow> {
        let token = self.http.resolve_token()?;
        let op = &ops::UPDATE_TEAM;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// DELETE /api/teams/{id} — soft-delete a team (owner or system admin), asking for what the
    /// delete did to `public_within` rows (`?answer=effects`).
    ///
    /// `Some` on a `200` with the effects, filtered for the caller; `None` on a `204`, which an
    /// older server that ignores the parameter answers: no effects reported, not none happened.
    /// `send` errors on any non-2xx (403/404/409).
    pub async fn delete(&self, team_id: Uuid) -> Result<Option<PublicWithinEffects>> {
        let token = self.http.resolve_token()?;
        let op = &ops::DELETE_TEAM;
        let path = op.path(&[&team_id])?;
        let req = self
            .http
            .request(op, &path)
            .query(&TeamActAnswerQuery::effects());
        let resp = self
            .http
            .send(&op.method(), &path, req, token.as_deref())
            .await?;
        effects_or_none(resp).await
    }

    /// DELETE /api/teams/{id}/children/{child_id} — detach a child team from this team, asking
    /// for what the detach did to `public_within` rows (`?answer=effects`).
    ///
    /// `Some` on a `200`, `None` on a `204` (an older server): see [`Self::delete`]. `send` errors on
    /// any non-2xx (403/404).
    pub async fn detach_child(
        &self,
        team_id: Uuid,
        child_id: Uuid,
    ) -> Result<Option<PublicWithinEffects>> {
        let token = self.http.resolve_token()?;
        let op = &ops::DETACH_CHILD_TEAM;
        let path = op.path(&[&team_id, &child_id])?;
        let req = self
            .http
            .request(op, &path)
            .query(&TeamActAnswerQuery::effects());
        let resp = self
            .http
            .send(&op.method(), &path, req, token.as_deref())
            .await?;
        effects_or_none(resp).await
    }

    /// PUT /api/teams/{id}/children/{child_id} — attach a child team under this team (a manager
    /// of both; a temper-system or gating parent, or a child with an auto-join or the gating team
    /// in or below it, live or deleted, a system admin).
    pub async fn attach_child(&self, team_id: Uuid, child_id: Uuid) -> Result<AttachChildOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::ATTACH_CHILD_TEAM;
        let path = op.path(&[&team_id, &child_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// POST /api/teams/{id}/reparent — move this team from one parent to another in one act.
    pub async fn reparent(&self, team_id: Uuid, body: &ReparentRequest) -> Result<ReparentOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::REPARENT_TEAM;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// POST /api/teams/{id}/restore — restore a soft-deleted team (its owner, or a system admin).
    pub async fn restore(&self, team_id: Uuid) -> Result<RestoreTeamOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::RESTORE_TEAM;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// POST /api/teams/{id}/rename-slug — change a team's slug. `+old/...` refs stop resolving.
    pub async fn rename_slug(
        &self,
        team_id: Uuid,
        body: &RenameTeamSlugRequest,
    ) -> Result<RenameTeamSlugOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::RENAME_TEAM_SLUG;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// PUT /api/teams/{id}/auto-join — set, change or clear a team's auto-join role (a system
    /// admin).
    pub async fn set_auto_join(
        &self,
        team_id: Uuid,
        body: &SetAutoJoinRequest,
    ) -> Result<SetAutoJoinOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::SET_TEAM_AUTO_JOIN;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// PUT /api/teams/{id}/public-within/{anchor_id} — open the team's owned contexts to the people
    /// under the anchor (a manager of the team, and a manager of the anchor or a system admin).
    pub async fn open_public_within(
        &self,
        team_id: Uuid,
        anchor_id: Uuid,
    ) -> Result<PublicWithinOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::OPEN_PUBLIC_WITHIN;
        let path = op.path(&[&team_id, &anchor_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// DELETE /api/teams/{id}/public-within/{anchor_id} — close the row (either side, or a system
    /// admin). Returns `()` on a 204.
    pub async fn close_public_within(&self, team_id: Uuid, anchor_id: Uuid) -> Result<()> {
        let token = self.http.resolve_token()?;
        let op = &ops::CLOSE_PUBLIC_WITHIN;
        let path = op.path(&[&team_id, &anchor_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send(&op.method(), &path, req, token.as_deref())
            .await?;
        Ok(())
    }

    /// PUT /api/teams/{id}/public-within-policy — set or clear the team's forbid (its owner or a
    /// system admin; on temper-system, the instance-wide off, a system admin only).
    pub async fn set_public_within_policy(
        &self,
        team_id: Uuid,
        body: &PublicWithinPolicyRequest,
    ) -> Result<PublicWithinPolicyOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::SET_PUBLIC_WITHIN_POLICY;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// GET /api/teams/public-within — the rows that confer on the caller right now.
    pub async fn list_public_within(&self) -> Result<Vec<PublicWithinRowRef>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_PUBLIC_WITHIN;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// PATCH /api/teams/{id}/members/{profile_id} — change a member's role.
    pub async fn change_role(
        &self,
        team_id: Uuid,
        profile_id: Uuid,
        body: &ChangeRoleRequest,
    ) -> Result<TeamMemberRow> {
        let token = self.http.resolve_token()?;
        let op = &ops::CHANGE_ROLE;
        let path = op.path(&[&team_id, &profile_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// DELETE /api/teams/{id}/members/{profile_id} — remove a member (or self-leave).
    ///
    /// Returns the residual owned-resource reach the removed member retains (the
    /// resources they still OWN in the team's contexts) so the caller can hand it
    /// off via `reassign`. `send` errors on any non-2xx (403/404/409), so callers
    /// still surface the guard failures.
    ///
    /// Skew-tolerant: an older server (pre-200 change) still returns `204 No Content`
    /// for a successful removal. An empty body decodes to an empty reach rather than a
    /// spurious decode error, so a newer CLI never reports a false failure against a
    /// lagging server — the removal still succeeded.
    pub async fn remove_member(
        &self,
        team_id: Uuid,
        profile_id: Uuid,
    ) -> Result<RemoveMemberOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::REMOVE_MEMBER;
        let path = op.path(&[&team_id, &profile_id])?;
        let req = self.http.request(op, &path);
        let resp = self
            .http
            .send(&op.method(), &path, req, token.as_deref())
            .await?;
        let bytes = resp.bytes().await?;
        if bytes.is_empty() {
            return Ok(RemoveMemberOutcome {
                residual_owned: ResidualOwnedReach {
                    count: 0,
                    contexts: Vec::new(),
                },
            });
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// POST /api/teams/{id}/invite — create a pending invitation.
    pub async fn invite(
        &self,
        team_id: Uuid,
        body: &CreateInvitationRequest,
    ) -> Result<TeamInvitation> {
        let token = self.http.resolve_token()?;
        let op = &ops::CREATE_TEAM_INVITATION;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// POST /api/teams/{id}/reassign — bulk-reassign the team's resources.
    pub async fn reassign(
        &self,
        team_id: Uuid,
        body: &BulkReassignRequest,
    ) -> Result<BulkReassignAck> {
        let token = self.http.resolve_token()?;
        let op = &ops::REASSIGN_TEAM;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// GET /api/teams/{id}/invitations — list pending invitations.
    pub async fn list_invitations(&self, team_id: Uuid) -> Result<Vec<TeamInvitation>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_TEAM_INVITATIONS;
        let path = op.path(&[&team_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// DELETE /api/teams/{id}/invitations/{invitation_id} — revoke a pending invite
    /// (a manager of the team, or a system admin). The invitation id is the one returned by `list_invitations`.
    pub async fn revoke_invitation(&self, team_id: Uuid, invitation_id: Uuid) -> Result<()> {
        let token = self.http.resolve_token()?;
        let op = &ops::REVOKE_TEAM_INVITATION;
        let path = op.path(&[&team_id, &invitation_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send(&op.method(), &path, req, token.as_deref())
            .await?;
        Ok(())
    }

    /// GET /api/invitations/mine — the caller's own pending invitations.
    pub async fn list_my_invitations(&self) -> Result<Vec<InviteeInvitation>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_MINE;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// GET /api/invitations/mine/count — how many invitations are waiting, without them.
    ///
    /// [`Self::list_my_invitations`] returns each invitation's redemption `token`; this returns
    /// integers. Callers that only need the number — `temper warmup`, which runs at every
    /// session start — take this one so no redemption token crosses the wire to produce a count.
    ///
    /// `team_slug` asks, in the same round trip, how many of them are to that team.
    /// `None` in, `matching: None` out: not asking and being told none are different answers.
    pub async fn count_my_invitations(
        &self,
        team_slug: Option<&str>,
    ) -> Result<PendingInvitationCounts> {
        let token = self.http.resolve_token()?;
        let op = &ops::COUNT_MINE;
        let path = op.path(&[])?;
        let mut req = self.http.request(op, &path);
        if let Some(slug) = team_slug {
            req = req.query(&[("team_slug", slug)]);
        }
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// POST /api/invitations/accept — redeem an invitation token. Only the invitee (the
    /// account whose verified email the invitation names) may; anyone else gets a 403.
    ///
    /// The invite token rides in the **body**, not the path: it is a private
    /// capability, and this client records `"{method} {path}"` as a span
    /// attribute that is exported. See `InvitationTokenRequest`.
    pub async fn accept_invitation(&self, invite_token: &str) -> Result<AcceptInvitationResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::ACCEPT;
        let path = op.path(&[])?;
        let body = InvitationTokenRequest {
            token: invite_token.to_string(),
        };
        let req = self.http.request(op, &path).json(&body);
        self.http
            .send_json(&op.method(), &path, req, token.as_deref())
            .await
    }

    /// POST /api/invitations/decline — decline an invitation token.
    ///
    /// Body-carried for the same reason as `accept_invitation`.
    /// Returns `()` on a 204; `send` errors on any non-2xx.
    pub async fn decline_invitation(&self, invite_token: &str) -> Result<()> {
        let token = self.http.resolve_token()?;
        let op = &ops::DECLINE;
        let path = op.path(&[])?;
        let body = InvitationTokenRequest {
            token: invite_token.to_string(),
        };
        let req = self.http.request(op, &path).json(&body);
        self.http
            .send(&op.method(), &path, req, token.as_deref())
            .await?;
        Ok(())
    }
}

/// A `?answer=effects` act's answer: the effects on a body, `None` on a `204` (a server that
/// predates the parameter answers as it always did).
async fn effects_or_none(resp: reqwest::Response) -> Result<Option<PublicWithinEffects>> {
    if resp.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
    }
    let bytes = resp.bytes().await?;
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}
